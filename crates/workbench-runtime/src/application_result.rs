use crate::app_bridge::{ApplicationModelTask, validate_tool_arguments};
use serde_json::{Value, json};

pub(crate) const RESULT_TOOL: &str = "cilai_submit_application_result";

pub(crate) fn validate_result_schema(schema: &Value) -> Result<(), String> {
    fn visit(schema: &Value, depth: usize) -> Result<(), String> {
        let object = schema.as_object().ok_or("结果结构必须是对象")?;
        if depth > 16 {
            return Err("结果结构嵌套过深".into());
        }
        for (key, value) in object {
            match key.as_str() {
                "type" => {
                    let supported = |name: &str| {
                        matches!(
                            name,
                            "object"
                                | "array"
                                | "string"
                                | "number"
                                | "integer"
                                | "boolean"
                                | "null"
                        )
                    };
                    let valid = value.as_str().is_some_and(supported)
                        || value.as_array().is_some_and(|types| {
                            !types.is_empty()
                                && types
                                    .iter()
                                    .all(|item| item.as_str().is_some_and(supported))
                        });
                    if !valid {
                        return Err("结果结构类型不受支持".into());
                    }
                }
                "properties" => {
                    for child in value.as_object().ok_or("字段结构不正确")?.values() {
                        visit(child, depth + 1)?;
                    }
                }
                "items" => visit(value, depth + 1)?,
                "allOf" => {
                    let constraints = value.as_array().ok_or("组合约束必须是数组")?;
                    if constraints.is_empty() {
                        return Err("组合约束不能为空".into());
                    }
                    for child in constraints {
                        visit(child, depth + 1)?;
                    }
                }
                "pattern" => {
                    let pattern = value.as_str().ok_or("格式约束必须是字符串")?;
                    regex::Regex::new(pattern).map_err(|_| "格式约束正则表达式无效")?;
                }
                "required" => {
                    let names = value.as_array().ok_or("必填字段不正确")?;
                    if names.iter().any(|name| {
                        name.as_str().is_none_or(|name| {
                            object
                                .get("properties")
                                .and_then(|props| props.get(name))
                                .is_none()
                        })
                    }) {
                        return Err("必填字段未声明".into());
                    }
                }
                "additionalProperties" | "uniqueItems" if value.is_boolean() => {}
                "minLength" | "maxLength" | "minItems" | "maxItems" if value.as_u64().is_some() => {
                }
                "minimum" | "maximum" if value.is_number() => {}
                "enum" if value.as_array().is_some_and(|items| !items.is_empty()) => {}
                "const" => {}
                "description" | "title" if value.is_string() => {}
                _ => return Err(format!("结果结构字段不受支持：{key}")),
            }
        }
        Ok(())
    }
    if schema.get("type").and_then(Value::as_str) != Some("object")
        || schema.to_string().len() > 32_768
    {
        return Err("结果结构必须是有限大小的对象".into());
    }
    visit(schema, 0)
}

pub(crate) fn tool(task: &ApplicationModelTask) -> Value {
    json!({
        "type": "function", "name": RESULT_TOOL,
        "description": "提交本轮应用任务的结构化候选结果。只接收当前任务及冻结输入，不保存或确认业务成果。必须成功提交后才能结束；普通最终回复不作为任务结果。",
        "inputSchema": {
            "type": "object", "additionalProperties": false,
            "properties": {
                "taskId": { "type": "string", "const": task.task_id },
                "inputDigest": { "type": "string", "const": task.input_digest },
                "result": task.result_schema
            },
            "required": ["taskId", "inputDigest", "result"]
        }
    })
}

pub(crate) fn handle(
    message: &Value,
    thread_id: &str,
    turn_id: &str,
    task: Option<&ApplicationModelTask>,
    accepted: &mut Option<String>,
    send: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<bool, String> {
    if message.get("method").and_then(Value::as_str) != Some("item/tool/call")
        || message.pointer("/params/tool").and_then(Value::as_str) != Some(RESULT_TOOL)
    {
        return Ok(false);
    }
    let id = message
        .get("id")
        .ok_or("application_result_request_id_missing")?;
    let submit = || -> Result<String, String> {
        let task = task
            .filter(|task| task.result_schema.is_some())
            .ok_or("本轮未声明结果提交协议")?;
        if message.pointer("/params/threadId").and_then(Value::as_str) != Some(thread_id)
            || message.pointer("/params/turnId").and_then(Value::as_str) != Some(turn_id)
        {
            return Err("结果不属于当前运行".into());
        }
        let args = message.pointer("/params/arguments").ok_or("缺少结果参数")?;
        validate_tool_arguments(&tool(task)["inputSchema"], args)?;
        let result = serde_json::to_string(&args["result"]).map_err(|error| error.to_string())?;
        if result.len() > 1_000_000 {
            return Err("结果超过大小限制".into());
        }
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
                "候选结果已接收，仍须应用业务校验和人工核对。".to_owned(),
            )
        }
        Err(error) => (false, error),
    };
    send(
        &json!({"id": id, "result": {"success": success, "contentItems": [{"type":"inputText","text":text}]}}),
    )?;
    Ok(true)
}

pub(crate) fn finish(
    task: Option<&ApplicationModelTask>,
    accepted: Option<String>,
    text: String,
) -> Result<String, String> {
    if task.is_some_and(|task| task.result_schema.is_some()) {
        accepted.ok_or_else(|| "application_result_missing:模型未提交结构化结果".into())
    } else {
        Ok(text)
    }
}

pub(crate) fn from_native_turn(
    task: &ApplicationModelTask,
    turn: &Value,
) -> Result<String, String> {
    let items = turn["items"]
        .as_array()
        .ok_or("application_result_history_invalid")?;
    let mut accepted: Option<String> = None;
    for item in items.iter().filter(|item| {
        item["type"] == "dynamicToolCall"
            && item["tool"] == RESULT_TOOL
            && item["status"] == "completed"
            && item["success"] == true
    }) {
        validate_tool_arguments(&tool(task)["inputSchema"], &item["arguments"])?;
        let result = serde_json::to_string(&item["arguments"]["result"])
            .map_err(|_| "application_result_history_invalid")?;
        if result.len() > 1_000_000 || accepted.as_ref().is_some_and(|old| old != &result) {
            return Err("application_result_history_conflict".into());
        }
        accepted = Some(result);
    }
    accepted.ok_or("application_result_history_missing".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> ApplicationModelTask {
        ApplicationModelTask {
            purpose: "test.compile".into(),
            task_id: "task".into(),
            input_digest: "digest".into(),
            result_schema: Some(
                json!({"type":"object","properties":{"markdown":{"type":"string","minLength":1}},"required":["markdown"],"additionalProperties":false}),
            ),
        }
    }
    #[test]
    fn recovery_requires_successful_submission_and_preserves_correction_after_rejection() {
        let task = task();
        let accepted = json!({"type":"dynamicToolCall","tool":RESULT_TOOL,"status":"completed","success":true,
            "arguments":{"taskId":"task","inputDigest":"digest","result":{"markdown":"正文"}}});
        let mut rejected = accepted.clone();
        rejected["success"] = json!(false);
        rejected["arguments"]["result"] = json!({});
        assert_eq!(
            from_native_turn(&task, &json!({"items":[rejected,accepted.clone()]})).unwrap(),
            r#"{"markdown":"正文"}"#
        );
        for success in [json!(false), Value::Null] {
            let mut unaccepted = accepted.clone();
            unaccepted["success"] = success;
            assert!(from_native_turn(&task, &json!({"items":[unaccepted]})).is_err());
        }
        let mut mismatched = accepted.clone();
        mismatched["arguments"]["inputDigest"] = json!("other");
        assert!(from_native_turn(&task, &json!({"items":[mismatched]})).is_err());
        let mut conflicting = accepted.clone();
        conflicting["arguments"]["result"]["markdown"] = json!("覆盖");
        assert!(from_native_turn(&task, &json!({"items":[accepted,conflicting]})).is_err());
    }

    #[test]
    fn result_channel_binds_run_and_schema_and_never_recovers_prose() {
        let task = task();
        let mut accepted = None;
        let mut responses = Vec::new();
        for (thread, digest, result, expected) in [
            ("other", "digest", json!({"markdown":"正文"}), false),
            ("thread", "other", json!({"markdown":"正文"}), false),
            ("thread", "digest", json!({}), false),
            ("thread", "digest", json!({"markdown":"正文"}), true),
            ("thread", "digest", json!({"markdown":"正文"}), true),
            ("thread", "digest", json!({"markdown":"覆盖"}), false),
        ] {
            let message = json!({"id":1,"method":"item/tool/call","params":{"threadId":thread,"turnId":"turn","tool":RESULT_TOOL,"arguments":{"taskId":"task","inputDigest":digest,"result":result}}});
            handle(
                &message,
                "thread",
                "turn",
                Some(&task),
                &mut accepted,
                &mut |value| {
                    responses.push(value.clone());
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(responses.last().unwrap()["result"]["success"], expected);
        }
        assert_eq!(
            finish(Some(&task), accepted, "已完成，以下是JSON".into()).unwrap(),
            "{\"markdown\":\"正文\"}"
        );
        assert!(finish(Some(&task), None, "{\"markdown\":\"正文\"}".into()).is_err());
        assert_eq!(finish(None, None, "普通回复".into()).unwrap(), "普通回复");
    }
    #[test]
    fn result_schema_rejects_unsupported_constraints() {
        assert!(validate_result_schema(task().result_schema.as_ref().unwrap()).is_ok());
        assert!(validate_result_schema(&json!({"type":"object","$ref":"remote"})).is_err());
        assert!(
            validate_result_schema(&json!({"type":"object","required":["undeclared"]})).is_err()
        );
        for schema in [
            json!({"type":"object","properties":{"text":{"type":"string","pattern":"["}}}),
            json!({"type":"object","allOf":[]}),
            json!({"type":"object","allOf":[{"$ref":"remote"}]}),
        ] {
            assert!(validate_result_schema(&schema).is_err());
        }
    }

    #[test]
    fn nullable_result_fields_preserve_strict_type_validation() {
        let schema = json!({
            "type":"object", "additionalProperties":false,
            "properties":{"published":{"type":["string","null"]}},
            "required":["published"]
        });
        validate_result_schema(&schema).unwrap();
        assert!(validate_tool_arguments(&schema, &json!({"published":null})).is_ok());
        assert!(validate_tool_arguments(&schema, &json!({"published":"2026-10-17"})).is_ok());
        for result in [
            json!({}),
            json!({"published":123}),
            json!({"published":false}),
        ] {
            assert!(validate_tool_arguments(&schema, &result).is_err());
        }
        for types in [
            json!([]),
            json!(["string", "unknown"]),
            json!(["string", null]),
        ] {
            assert!(
                validate_result_schema(&json!({
                    "type":"object","properties":{"published":{"type":types}}
                }))
                .is_err()
            );
        }
    }

    #[test]
    fn rejected_field_constraints_can_be_corrected_before_result_is_accepted() {
        let mut task = task();
        task.result_schema = Some(json!({
            "type":"object", "additionalProperties":false, "required":["text"],
            "properties":{"text":{"type":"string","allOf":[
                {"pattern":"^[^0-9０-９]*$","description":"正文不得自行填写数字"},
                {"pattern":"依据","description":"正文须包含依据"}
            ]}}
        }));
        validate_result_schema(task.result_schema.as_ref().unwrap()).unwrap();
        let mut accepted = None;
        for (text, expected, detail) in [
            ("依据包含3项", false, "正文不得自行填写数字"),
            ("待核实", false, "正文须包含依据"),
            ("依据待核实", true, "候选结果已接收"),
        ] {
            let message = json!({"id":1,"method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","tool":RESULT_TOOL,"arguments":{"taskId":"task","inputDigest":"digest","result":{"text":text}}}});
            let mut response = Value::Null;
            handle(
                &message,
                "thread",
                "turn",
                Some(&task),
                &mut accepted,
                &mut |value| {
                    response = value.clone();
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(response["result"]["success"], expected);
            let message = response["result"]["contentItems"][0]["text"]
                .as_str()
                .unwrap();
            assert!(message.contains(detail));
            if !expected {
                assert!(message.contains("$.result.text"));
                assert!(accepted.is_none());
            }
        }
        assert_eq!(
            finish(Some(&task), accepted, String::new()).unwrap(),
            r#"{"text":"依据待核实"}"#
        );
    }
}
