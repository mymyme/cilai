use crate::codex_harness::CodexHarnessClassificationRequest;
use serde_json::{Value, json};

pub(crate) const RESULT_TOOL: &str = "cilai_submit_workbench_result";

pub(crate) fn tool(contract: &CodexHarnessClassificationRequest) -> Value {
    json!({"type":"function","name":RESULT_TOOL,
        "description":"完成实际工作后，提交本轮完整回复及工作归类；实际文件成果须先登记，无文件成果的只读回答仍须正式提交。必须成功提交后才能结束；普通最终正文不能代替正式结果。校验失败时根据错误修正后重新提交。",
        "inputSchema":contract.output_schema})
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
    let submit = || -> Result<String, String> {
        let contract = contract.ok_or("本轮未声明主对话结果协议")?;
        if message["params"]["threadId"] != thread || message["params"]["turnId"] != turn {
            return Err("结果不属于当前运行".into());
        }
        let args = message["params"].get("arguments").ok_or("缺少结果参数")?;
        crate::app_bridge::validate_tool_arguments(&contract.output_schema, args).map_err(|error| {
            if let Some(expected) = contract.output_schema.pointer("/properties/nonce/enum/0")
                && args.get("nonce") != Some(expected)
            {
                return format!(
                    "{error}。当前本轮 nonce 的唯一允许值是 {expected}，请原样使用该值重新提交完整结果；无需重复已完成的业务工具或成果登记。错误结果尚未接收。"
                );
            }
            error
        })?;
        let result = serde_json::to_string(args).map_err(|e| e.to_string())?;
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
        Ok(result)
    };
    let (success, text) = match submit() {
        Ok(result) => {
            *accepted = Some(result);
            (
                true,
                "本轮正式结果已接收，业务状态由工作台统一提交。".to_owned(),
            )
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

pub(crate) fn from_native_turn(turn: &Value, nonce: &str) -> Result<String, String> {
    let items = turn["items"]
        .as_array()
        .ok_or("workbench_result_history_invalid")?;
    let mut accepted = None;
    for item in items.iter().filter(|item| {
        item["type"] == "dynamicToolCall"
            && item["tool"] == RESULT_TOOL
            && item["status"] == "completed"
            && item["success"] == true
    }) {
        let args = &item["arguments"];
        if args["schemaVersion"] != 1 || args["nonce"] != nonce || !args["reply"].is_string() {
            return Err("workbench_result_history_scope_invalid".into());
        }
        let result = serde_json::to_string(args).map_err(|_| "workbench_result_history_invalid")?;
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
