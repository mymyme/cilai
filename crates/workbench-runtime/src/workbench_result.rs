use crate::codex_harness::CodexHarnessClassificationRequest;
use serde_json::{Value, json};
use std::collections::HashSet;

pub(crate) const TOOL_CONFIG_VERSION: i64 = 5;

pub(crate) const RESULT_TOOL: &str = "cilai_submit_workbench_result";

#[path = "work_execution_result.rs"]
pub(crate) mod execution;

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkInputRequest {
    pub question: String,
    pub impact: String,
}

pub(crate) const HOST_BOUND_PROTOCOL_VERSION: u32 = 8;
const CONTRACT_PREFIX: &str = "cilai_workbench_result_";
const LEGACY_CONTRACT_KEY: &str = "cilai_response_contract";
const ACCEPTED: &str = "本轮正式结果已接收，业务状态由工作台统一提交。";
const RECEIPT_PREFIX: &str = "cilai_workbench_result_receipt:";

pub(crate) fn contract_key(scope: &str) -> String {
    format!("{CONTRACT_PREFIX}{scope}")
}

const DESCRIPTION: &str = "完成实际工作后，提交本轮完整回复及工作归类；实际文件成果须先登记。按本轮要求提交，普通正文不能代替正式结果。";

pub(crate) fn model_schema() -> Value {
    let mut schema = crate::ai_runtime::model_classification_output_schema("", Some("current"));
    schema["properties"]
        .as_object_mut()
        .expect("result properties")
        .remove("nonce");
    schema["required"]
        .as_array_mut()
        .expect("result required")
        .retain(|field| field != "nonce");
    schema
}

pub(crate) fn tool() -> Value {
    json!({"type":"function","name":RESULT_TOOL,"description":DESCRIPTION,"inputSchema":model_schema()})
}

pub(crate) fn project_model_tool(tool: &mut Value) -> bool {
    let schema = model_schema();
    if tool["description"] == DESCRIPTION && tool["parameters"] == schema {
        return false;
    }
    tool["description"] = json!(DESCRIPTION);
    tool["parameters"] = schema;
    true
}

pub(crate) struct ReplyStream {
    tool: &'static str,
    field: &'static str,
    call: Option<ReplyCall>,
}

struct ReplyCall {
    item_id: String,
    call_id: Option<String>,
    arguments: String,
    published_len: usize,
    dirty: bool,
    closed: bool,
}

impl Default for ReplyStream {
    fn default() -> Self {
        Self::for_tool(RESULT_TOOL, "reply")
    }
}

impl ReplyStream {
    pub(crate) fn for_tool(tool: &'static str, field: &'static str) -> Self {
        Self {
            tool,
            field,
            call: None,
        }
    }

    pub(crate) fn call_id(&self) -> Option<&str> {
        self.call.as_ref()?.call_id.as_deref()
    }

    pub(crate) fn observe(&mut self, event: &Value) {
        self.push(event);
    }

    fn push(&mut self, event: &Value) -> Option<()> {
        let item = &event["item"];
        let kind = event["type"].as_str()?;
        if kind == "response.output_item.added"
            && item["type"] == "function_call"
            && item["name"] == self.tool
            && item["namespace"].is_null()
            && self.call.is_none()
        {
            self.call = Some(ReplyCall {
                item_id: item["id"].as_str()?.to_owned(),
                call_id: item["call_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned),
                arguments: String::new(),
                published_len: 0,
                dirty: false,
                closed: false,
            });
        }
        let call = self.call.as_mut()?;
        if call.closed {
            return None;
        }
        let next = match kind {
            "response.output_item.added" if item["id"] == call.item_id => {
                item["arguments"].as_str()?
            }
            "response.function_call_arguments.delta" if event["item_id"] == call.item_id => {
                event["delta"].as_str()?
            }
            "response.output_item.done" if item["id"] == call.item_id => {
                call.closed = true;
                return None;
            }
            _ => return None,
        };
        if next.is_empty() {
            return None;
        }
        if call.arguments.len().saturating_add(next.len()) > 1_000_000 {
            call.closed = true;
            call.arguments.clear();
            call.dirty = false;
            return None;
        }
        call.arguments.push_str(next);
        call.dirty = true;
        Some(())
    }

    pub(crate) fn take_preview(&mut self) -> Option<String> {
        let call = self.call.as_mut()?;
        if !std::mem::take(&mut call.dirty) {
            return None;
        }
        if !call.arguments.trim_start().starts_with('{') {
            return None;
        }
        let text = crate::work_classification::field_prefix(&call.arguments, self.field)?;
        let visible = if self.field == "reply" {
            crate::work_classification::visible_reply(&text, "", true).ok()??
        } else {
            text
        };
        if visible.len() <= call.published_len || visible.trim().is_empty() {
            return None;
        }
        call.published_len = visible.len();
        Some(visible)
    }
}

pub(crate) fn receipt_text(output: &Value) -> Option<&str> {
    match output {
        Value::String(text) => Some(text.as_str()),
        Value::Array(parts) if parts.len() == 1 && parts[0]["type"] == "input_text" => {
            parts[0]["text"].as_str()
        }
        _ => None,
    }
}

pub(crate) fn reply_already_accepted(input: &[Value]) -> bool {
    let tail = || input.iter().rev().take_while(|item| item["role"] != "user");
    tail().any(|item| {
        item["type"] == "function_call_output"
            && receipt_text(&item["output"]) == Some(ACCEPTED)
            && item["call_id"].as_str().is_some_and(|id| {
                tail().any(|call| {
                    call["type"] == "function_call"
                        && call["name"] == RESULT_TOOL
                        && call["namespace"].is_null()
                        && call["call_id"] == id
                })
            })
    })
}

fn contract_fragment(key: &str, base: &str) -> bool {
    key == base
        || key
            .strip_prefix(base)
            .and_then(|part| part.strip_prefix("_part_"))
            .is_some_and(|part| part.len() == 6 && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_expired_contract(part: &Value, current: &str) -> bool {
    if part["type"] != "input_text" {
        return false;
    }
    let Some((key, text)) = part["text"]
        .as_str()
        .and_then(|text| text.strip_prefix('<'))
        .and_then(|text| text.split_once('>'))
    else {
        return false;
    };
    if !text.ends_with(&format!("</{key}>")) || contract_fragment(key, current) {
        return false;
    }
    if contract_fragment(key, LEGACY_CONTRACT_KEY) {
        return true;
    }
    let Some(scope) = key.strip_prefix(CONTRACT_PREFIX) else {
        return false;
    };
    let base = scope.split_once("_part_").map_or(scope, |(base, _)| base);
    base.strip_prefix("run_")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        && contract_fragment(scope, base)
}

pub(crate) fn project_model_history(request: &mut Value, scope: &str) -> bool {
    let Some(input) = request.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let current = contract_key(scope);
    let result_calls: HashSet<String> = input
        .iter()
        .filter(|item| {
            item["type"] == "function_call"
                && item["name"] == RESULT_TOOL
                && item["namespace"].is_null()
        })
        .filter_map(|item| item["call_id"].as_str().map(str::to_owned))
        .collect();
    let mut changed = false;
    input.retain_mut(|item| {
        if item["type"] == "message"
            && item["role"] == "developer"
            && let Some(content) = item.get_mut("content").and_then(Value::as_array_mut)
        {
            let before = content.len();
            content.retain(|part| !is_expired_contract(part, &current));
            if content.len() != before {
                changed = true;
                return !content.is_empty();
            }
        }
        if !item["call_id"]
            .as_str()
            .is_some_and(|id| result_calls.contains(id))
        {
            return true;
        }
        if item["type"] == "function_call"
            && let Some(raw) = item["arguments"].as_str()
            && let Ok(mut args) = serde_json::from_str::<Value>(raw)
            && args
                .as_object_mut()
                .and_then(|args| args.remove("nonce"))
                .is_some()
        {
            item["arguments"] = json!(args.to_string());
            changed = true;
        } else if item["type"] == "function_call_output"
            && let Some(output) = receipt_text(&item["output"])
        {
            let projected = if output.starts_with(ACCEPTED) && output.contains(RECEIPT_PREFIX) {
                Some(ACCEPTED)
            } else if output.contains("当前本轮 nonce 的唯一允许值是") {
                Some("该轮正式结果提交曾被拒绝。")
            } else {
                None
            };
            if let Some(projected) = projected {
                if item["output"].is_array() {
                    item["output"][0]["text"] = json!(projected);
                } else {
                    item["output"] = json!(projected);
                }
                changed = true;
            }
        }
        true
    });
    changed
}

fn bind_arguments(args: &Value, nonce: &str) -> Result<Value, String> {
    let mut bound = args.as_object().cloned().ok_or("缺少结果参数")?;
    if bound.contains_key("nonce") {
        return Err("正式结果的校验标识由工作台生成。".into());
    }
    bound.insert("nonce".into(), json!(nonce));
    Ok(Value::Object(bound))
}

fn receipt(call_id: &str, result: &Value) -> String {
    let digest = crate::persistent_threads::digest(&json!([call_id, result]));
    format!("{ACCEPTED}\n{RECEIPT_PREFIX}{digest}")
}

pub(crate) fn handle(
    message: &Value,
    thread: &str,
    turn: &str,
    contract: Option<&CodexHarnessClassificationRequest>,
    accepted: &mut Option<String>,
    send: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<bool, String> {
    if message["method"] != "item/tool/call" || message["params"]["tool"] != RESULT_TOOL {
        return Ok(false);
    }
    let id = message
        .get("id")
        .ok_or("workbench_result_request_id_missing")?;
    let submit = || -> Result<(String, String), String> {
        let contract = contract.ok_or("本轮未声明主对话结果协议")?;
        let params = &message["params"];
        if params["threadId"] != thread
            || params["turnId"] != turn
            || !params["namespace"].is_null()
        {
            return Err("结果不属于当前运行".into());
        }
        let call_id = params["callId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("缺少原生调用编号")?;
        let nonce = contract
            .output_schema
            .pointer("/properties/nonce/enum/0")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or("缺少本轮校验标识")?;
        let args = bind_arguments(&params["arguments"], nonce)?;
        crate::app_bridge::validate_tool_arguments(&contract.output_schema, &args)?;
        let result = args.to_string();
        if result.len() > 1_000_000 {
            return Err("结果超过大小限制".into());
        }
        (contract.validate)(&result)?;
        if accepted
            .as_ref()
            .is_some_and(|previous| previous != &result)
        {
            return Err("本轮已提交其他结果，请新建运行".into());
        }
        Ok((result, receipt(call_id, &args)))
    };
    let (success, text) = match submit() {
        Ok((result, receipt)) => {
            *accepted = Some(result);
            (true, receipt)
        }
        Err(error) => (false, error),
    };
    send(
        &json!({"id":id,"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}}),
    )?;
    Ok(true)
}

pub(crate) fn reply(result: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(result).map_err(|_| "workbench_result_invalid")?;
    value["reply"]
        .as_str()
        .filter(|reply| !reply.trim().is_empty())
        .map(str::to_owned)
        .ok_or("workbench_result_reply_missing".into())
}

pub(crate) fn is_accepted_item(item: &Value) -> bool {
    item["type"] == "dynamicToolCall"
        && item["tool"] == RESULT_TOOL
        && item["namespace"].is_null()
        && item["status"] == "completed"
        && item["success"] == true
        && item["arguments"]
            .get(crate::public_reasoning::FIELD)
            .is_none()
}

pub(crate) fn from_native_turn(
    turn: &Value,
    nonce: &str,
    host_bound: bool,
) -> Result<String, String> {
    let items = turn["items"]
        .as_array()
        .ok_or("workbench_result_history_invalid")?;
    let mut accepted = None;
    for item in items.iter().filter(|item| is_accepted_item(item)) {
        let args = if host_bound {
            let args = bind_arguments(&item["arguments"], nonce)
                .map_err(|_| "workbench_result_history_scope_invalid")?;
            let call_id = item["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or("workbench_result_history_scope_invalid")?;
            let proof = receipt(call_id, &args);
            if item["contentItems"] != json!([{"type":"inputText","text":proof}]) {
                return Err("workbench_result_history_scope_invalid".into());
            }
            args
        } else {
            item["arguments"].clone()
        };
        if args["schemaVersion"] != 1 || args["nonce"] != nonce || !args["reply"].is_string() {
            return Err("workbench_result_history_scope_invalid".into());
        }
        let result = args.to_string();
        if result.len() > 1_000_000
            || accepted
                .as_ref()
                .is_some_and(|previous| previous != &result)
        {
            return Err("workbench_result_history_conflict".into());
        }
        accepted = Some(result);
    }
    accepted.ok_or("workbench_result_history_missing".into())
}

#[cfg(test)]
#[path = "workbench_result_tests.rs"]
mod tests;
