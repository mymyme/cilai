use crate::app_bridge::validate_tool_arguments;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

const TOOL_IDENTITY: &str = "host-report-review\nsubmit";

#[derive(Debug, Default)]
struct SubmissionState {
    accepted: Option<String>,
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReportResultContract {
    schema_version: u8,
    client_message_id: String,
    input_digest: String,
    report_id: String,
    version_id: String,
    #[serde(skip)]
    state: Arc<Mutex<SubmissionState>>,
}

impl ReportResultContract {
    pub(crate) fn output_schema(&self) -> Value {
        json!({
            "type":"object", "additionalProperties":false,
            "required":["schemaVersion","clientMessageId","inputDigest","reportId","versionId","result"],
            "properties":{
                "schemaVersion":{"type":"integer","enum":[self.schema_version]},
                "clientMessageId":{"type":"string","enum":[self.client_message_id]},
                "inputDigest":{"type":"string","enum":[self.input_digest]},
                "reportId":{"type":"string","enum":[self.report_id]},
                "versionId":{"type":"string","enum":[self.version_id]},
                "result":{
                    "type":"object","additionalProperties":false,
                    "required":["answer","html","edits"],
                    "properties":{
                        "answer":{"type":"string","minLength":1,"maxLength":100_000},
                        "html":{"type":["string","null"],"maxLength":800_000},
                        "edits":{"type":"array","maxItems":100,"items":{
                            "type":"object","additionalProperties":false,
                            "required":["search","replace"],
                            "properties":{
                                "search":{"type":"string","minLength":1,"maxLength":800_000},
                                "replace":{"type":"string","maxLength":800_000}
                            }
                        }}
                    }
                }
            }
        })
    }

    pub(crate) fn finish(&self, output: &str) -> Result<String, String> {
        if !matches!(self.schema_version, 1 | 2) || output.len() > 1_000_000 {
            return Err("report_result_invalid:报告候选结果无效".into());
        }
        let value: Value = serde_json::from_str(output)
            .map_err(|_| "report_result_invalid:模型未返回完整结构化报告结果")?;
        validate_tool_arguments(&self.output_schema(), &value)
            .map_err(|_| "report_result_invalid:报告结果与本轮输入或结果结构不符")?;
        let result = &value["result"];
        if result["answer"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
            || (result["html"].is_string() && !result["edits"].as_array().unwrap().is_empty())
            || result["html"]
                .as_str()
                .is_some_and(|text| text.trim().is_empty())
        {
            return Err("report_result_invalid:报告说明或修改方式无效".into());
        }
        serde_json::to_string(result).map_err(|_| "report_result_invalid".into())
    }

    pub(crate) fn tool_definition(&self) -> Value {
        json!({"title":"提交报告核实结果","description":"提交本轮政策报告核实结果。含修改时必须先在本轮用read_report和read_report_context按nextOffset读完；search从实际HTML逐字复制且唯一。工具拒绝时在本轮纠正，成功提交后结束；普通最终文字不作为候选。本工具只接收候选，不应用报告或写入企业业务。","inputSchema":self.output_schema(),"annotations":{"readOnlyHint":true,"destructiveHint":false}})
    }

    pub(crate) fn tool_name(&self) -> String {
        crate::application_tool_adapter::versioned_name(TOOL_IDENTITY, &self.tool_definition())
    }

    pub(crate) fn add_to_adapter(
        &self,
        adapter: &mut crate::application_tool_adapter::ApplicationToolAdapter,
    ) -> Result<(), String> {
        adapter.add_report_result(TOOL_IDENTITY, self.clone())
    }

    pub(crate) fn submit(&self, arguments: &Value) -> Result<Value, String> {
        if self.schema_version != 2 {
            return Err("报告提交协议无效".into());
        }
        let result = self.finish(&arguments.to_string())?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "report_result_state_invalid")?;
        let changes = arguments["result"]["html"].is_string()
            || arguments["result"]["edits"]
                .as_array()
                .is_some_and(|edits| !edits.is_empty());
        if changes {
            return Err("enterprise_report_review_removed".into());
        }
        if state
            .accepted
            .as_ref()
            .is_some_and(|previous| previous != &result)
        {
            return Err("本轮已接收其他候选，请另行发起请求".into());
        }
        state.accepted = Some(result.clone());
        let mut receipt = serde_json::to_value(self).map_err(|_| "report_result_invalid")?;
        receipt["kind"] = json!("cilai.report_result");
        receipt["accepted"] = json!(true);
        receipt["resultSha256"] = json!(digest(&result));
        receipt["reportHtmlSha256"] = Value::Null;
        receipt["reportContextSha256"] = Value::Null;
        Ok(
            json!({"content":[{"type":"text","text":"本轮核实结果已接收，报告修改仍须预览和人工应用。"}],"structuredContent":receipt,"isError":false}),
        )
    }

    pub(crate) fn complete(&self, legacy_output: &str) -> Result<String, String> {
        if self.schema_version == 1 {
            return self.finish(legacy_output);
        }
        self.state
            .lock()
            .map_err(|_| "report_result_state_invalid")?
            .accepted
            .clone()
            .ok_or_else(|| "report_result_missing:模型未通过正式工具提交报告核实结果".into())
    }

    pub(crate) fn recover(
        metadata: &Value,
        client_message_id: &str,
        input_digest: &str,
        turn: &Value,
    ) -> Result<String, String> {
        let contract: Self = serde_json::from_value(metadata.clone())
            .map_err(|_| "report_result_history_invalid")?;
        if contract.client_message_id != client_message_id || contract.input_digest != input_digest
        {
            return Err("report_result_history_invalid:报告结果不属于原请求".into());
        }
        if contract.schema_version == 1 {
            return crate::codex_threads::turn_final_reply(turn)
                .ok_or_else(|| "report_result_history_invalid".to_owned())
                .and_then(|output| contract.finish(&output));
        }
        if contract.schema_version != 2 {
            return Err("report_result_history_invalid".into());
        }
        let mut accepted = None;
        for item in turn["items"]
            .as_array()
            .ok_or("report_result_history_invalid")?
            .iter()
            .filter(|item| {
                item["type"] == "mcpToolCall"
                    && item["server"] == "cilai_application"
                    && item["tool"] == contract.tool_name()
                    && item["status"] == "completed"
                    && item["error"].is_null()
            })
        {
            let receipt = &item["result"]["structuredContent"];
            if receipt["accepted"] != true || receipt["kind"] != "cilai.report_result" {
                continue;
            }
            let result = contract.finish(&item["arguments"].to_string())?;
            for field in [
                "schemaVersion",
                "clientMessageId",
                "inputDigest",
                "reportId",
                "versionId",
            ] {
                if receipt[field] != metadata[field] {
                    return Err("report_result_history_invalid:报告回执绑定不一致".into());
                }
            }
            if receipt["resultSha256"] != digest(&result)
                || accepted
                    .as_ref()
                    .is_some_and(|previous| previous != &result)
            {
                return Err("report_result_history_invalid:报告结果摘要或回执冲突".into());
            }
            let changes = item["arguments"]["result"]["html"].is_string()
                || item["arguments"]["result"]["edits"]
                    .as_array()
                    .is_some_and(|edits| !edits.is_empty());
            if changes
                && ["reportHtmlSha256", "reportContextSha256"]
                    .iter()
                    .any(|field| {
                        receipt[field].as_str().is_none_or(|s| {
                            s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                    })
            {
                return Err("report_result_history_invalid:报告回执缺少本轮读取依据".into());
            }
            accepted = Some(result);
        }
        accepted.ok_or_else(|| "report_result_history_missing:未取得正式报告提交成功回执".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> ReportResultContract {
        ReportResultContract {
            schema_version: 1,
            client_message_id: "client".into(),
            input_digest: "digest".into(),
            report_id: uuid::Uuid::new_v4().to_string(),
            version_id: uuid::Uuid::new_v4().to_string(),
            state: Arc::default(),
        }
    }

    fn output(contract: &ReportResultContract) -> Value {
        let mut value = serde_json::to_value(contract).unwrap();
        value["result"] = json!({"answer":"已核实","html":null,"edits":[]});
        value
    }

    #[test]
    fn report_results_require_complete_bound_json_and_mutually_exclusive_changes() {
        let contract = contract();
        let valid = output(&contract);
        assert_eq!(
            serde_json::from_str::<Value>(&contract.finish(&valid.to_string()).unwrap()).unwrap(),
            valid["result"]
        );
        for raw in [
            format!("说明{valid}"),
            format!("```json\n{valid}\n```"),
            "普通答案".into(),
        ] {
            assert!(contract.finish(&raw).is_err());
        }
        for field in ["clientMessageId", "inputDigest", "reportId", "versionId"] {
            let mut invalid = valid.clone();
            invalid[field] = json!("other");
            assert!(contract.finish(&invalid.to_string()).is_err());
        }
        let mut invalid = valid.clone();
        invalid["result"]["edits"] = json!([{"search":"原文","replace":"新文"}]);
        assert!(contract.finish(&invalid.to_string()).is_ok());
        invalid["result"]["html"] = json!("<html>全文</html>");
        assert!(contract.finish(&invalid.to_string()).is_err());
        invalid["result"]["edits"] = json!([]);
        assert!(contract.finish(&invalid.to_string()).is_ok());
        invalid["result"]["answer"] = json!("   ");
        assert!(contract.finish(&invalid.to_string()).is_err());
    }

    #[test]
    fn report_recovery_uses_persisted_contract_and_original_request_identity() {
        let contract = contract();
        let metadata = serde_json::to_value(&contract).unwrap();
        let turn = json!({"items":[{"type":"agentMessage","phase":"final_answer","text":output(&contract).to_string()}]});
        assert!(ReportResultContract::recover(&metadata, "client", "digest", &turn).is_ok());
        assert!(ReportResultContract::recover(&metadata, "other", "digest", &turn).is_err());
        assert!(ReportResultContract::recover(&metadata, "client", "other", &turn).is_err());
        let mut invalid = metadata;
        invalid["schemaVersion"] = json!(2);
        assert!(ReportResultContract::recover(&invalid, "client", "digest", &turn).is_err());
    }

    fn submission() -> ReportResultContract {
        let mut contract = contract();
        contract.schema_version = 2;
        contract
    }

    #[test]
    fn removed_report_reads_cannot_authorize_new_changes() {
        let contract = submission();
        let mut candidate = output(&contract);
        candidate["result"]["edits"] = json!([{"search":"原文","replace":"核实稿"}]);
        assert_eq!(
            contract.submit(&candidate).unwrap_err(),
            "enterprise_report_review_removed"
        );
        assert!(contract.complete(&candidate.to_string()).is_err());
        candidate["result"]["edits"] = json!([]);
        assert!(contract.submit(&candidate).is_ok());
        assert_eq!(
            contract.complete("").unwrap(),
            candidate["result"].to_string()
        );
    }

    #[test]
    fn native_recovery_requires_matching_success_receipt_after_rejection_and_rejects_tampering() {
        let contract = submission();
        let mut candidate = output(&contract);
        candidate["result"]["edits"] = json!([{"search":"原文","replace":"核实稿"}]);
        let mut historical_receipt = serde_json::to_value(&contract).unwrap();
        historical_receipt["kind"] = json!("cilai.report_result");
        historical_receipt["accepted"] = json!(true);
        historical_receipt["resultSha256"] = json!(digest(&candidate["result"].to_string()));
        historical_receipt["reportHtmlSha256"] = json!(digest("<p>原文</p>"));
        historical_receipt["reportContextSha256"] = json!(digest("{}"));
        let accepted = json!({"structuredContent":historical_receipt});
        let metadata = serde_json::to_value(&contract).unwrap();
        let item = json!({"type":"mcpToolCall","server":"cilai_application","tool":contract.tool_name(),"status":"completed","arguments":candidate,"result":accepted,"error":null});
        let rejected = json!({"type":"mcpToolCall","server":"cilai_application","tool":contract.tool_name(),"status":"completed","arguments":candidate,"result":{"content":[{"type":"text","text":"拒绝"}]},"error":null});
        let turn = json!({"items":[rejected,item]});
        assert_eq!(
            ReportResultContract::recover(&metadata, "client", "digest", &turn).unwrap(),
            candidate["result"].to_string()
        );
        for pointer in [
            "/items/1/result/structuredContent/resultSha256",
            "/items/1/result/structuredContent/clientMessageId",
            "/items/1/result/structuredContent/reportHtmlSha256",
        ] {
            let mut invalid = turn.clone();
            *invalid.pointer_mut(pointer).unwrap() = json!("changed");
            assert!(
                ReportResultContract::recover(&metadata, "client", "digest", &invalid).is_err()
            );
        }
        let mut wrong_tool = turn.clone();
        wrong_tool["items"][1]["tool"] = json!("foreign");
        assert!(ReportResultContract::recover(&metadata, "client", "digest", &wrong_tool).is_err());
        assert!(ReportResultContract::recover(&metadata, "client", "digest", &json!({"items":[{"type":"agentMessage","phase":"final_answer","text":candidate.to_string()}]})).is_err());
    }
}
