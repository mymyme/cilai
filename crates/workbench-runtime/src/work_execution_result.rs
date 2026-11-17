use super::{CodexHarnessClassificationRequest, RESULT_TOOL, Value, WorkInputRequest, json};
use crate::codex_harness::ArtifactCallback;

const FIELD: &str = "waitingForInput";
const ARTIFACTS: &str = "artifactPaths";
const PREFIX: &str = "cilai_execution_result_";
const INSTRUCTIONS: &str = "本轮执行结束时，必须调用 cilai_submit_workbench_result 提交完整 reply、waitingForInput 和 artifactPaths；普通正文不能代替正式结果。waitingForInput 必须明确填写：若交付所必需的信息尚待用户提供或确认，填写包含 question 和 impact 的对象；无必需缺项则填 JSON null。用户要求准确的日期、地点、人员、金额等但未提供时，不得自行假定，不得以待确认文字、占位符或未填完的草稿当成完成。可说明已有进展，但仍须登记问题与影响。仅在正文提问不等于登记。用户明确允许留空的草稿、可选建议和普通问答不作为阻塞。artifactPaths 必须明确列出本轮实际交付文件的工作区路径，无文件则填空数组；宿主会逐一核验并登记，不能只在正文给出路径。收到结果回执后结束，不继续业务操作。";

pub(crate) fn context_key(scope: &str) -> String {
    format!("{PREFIX}{scope}")
}

pub(crate) fn instructions() -> &'static str {
    INSTRUCTIONS
}

pub(crate) fn declared(request: &Value, scope: &str) -> bool {
    let prefix = format!("<{}>", context_key(scope));
    request["input"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["role"] == "developer"
                && item["content"].as_array().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        part["type"] == "input_text"
                            && part["text"]
                                .as_str()
                                .is_some_and(|text| text.starts_with(&prefix))
                    })
                })
        })
    })
}

fn with_execution(mut schema: Value) -> Value {
    schema["properties"][FIELD] = json!({"anyOf":[
        {"type":"null"},
        {"type":"object","additionalProperties":false,"required":["question","impact"],"properties":{
            "question":{"type":"string","minLength":1,"maxLength":500},
            "impact":{"type":"string","minLength":1,"maxLength":500}
        }}
    ],"description":"交付仍需用户确认的必需信息与影响；无必需缺项填写 null，不能省略。"});
    schema["properties"][ARTIFACTS] = json!({"type":"array","maxItems":64,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":4096},"description":"本轮实际交付的已有文件路径。无文件填写空数组；宿主核验登记成功后才接受交付。"});
    schema["required"]
        .as_array_mut()
        .expect("result fields")
        .extend([json!(FIELD), json!(ARTIFACTS)]);
    schema
}

pub(crate) fn schema(classified: bool) -> Value {
    with_execution(if classified {
        super::model_schema()
    } else {
        json!({
            "type":"object","additionalProperties":false,"required":["schemaVersion","reply"],
            "properties":{"schemaVersion":{"type":"integer","enum":[1]},"reply":{"type":"string","minLength":1}}
        })
    })
}

pub(crate) fn tool(classified: bool) -> Value {
    let mut tool = super::tool();
    tool["inputSchema"] = schema(classified);
    tool
}

pub(crate) fn without_execution(output: &str) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(output).map_err(|_| "workbench_result_invalid")?;
    let fields = value.as_object_mut().ok_or("workbench_result_invalid")?;
    fields.remove(FIELD);
    fields.remove(ARTIFACTS);
    Ok(value.to_string())
}

pub(crate) fn project(request: &mut Value, classified: bool) -> Result<(), ()> {
    fn tools(container: &mut Value, schema: &Value) -> bool {
        let Some(tools) = container.get_mut("tools").and_then(Value::as_array_mut) else {
            return false;
        };
        let mut found = false;
        for tool in tools {
            if tool["type"] == "function" && tool["name"] == RESULT_TOOL {
                tool["parameters"] = schema.clone();
                tool["description"] = json!(
                    "完成本轮工作后提交完整回复、必需缺项状态和实际交付文件路径。waitingForInput 无缺项填 null，有缺项填写问题与影响；artifactPaths 无文件填空数组，有文件填写工作区路径，由宿主核验登记。此回执决定交付或等待，普通正文不能代替。"
                );
                found = true;
            }
        }
        found
    }
    let accepted = request["input"]
        .as_array()
        .is_some_and(|items| super::reply_already_accepted(items));
    let schema = schema(classified);
    let mut found = tools(request, &schema);
    if let Some(items) = request["input"].as_array_mut() {
        for item in items.iter_mut().filter(|item| {
            matches!(
                item["type"].as_str(),
                Some("additional_tools" | "tool_search_output")
            )
        }) {
            found |= tools(item, &schema);
        }
        if accepted {
            items.retain_mut(|item| match item["type"].as_str() {
                Some("additional_tools") => false,
                Some("tool_search_output") => {
                    item["tools"] = json!([]);
                    true
                }
                _ => true,
            });
        }
    }
    if !accepted && !found {
        return Err(());
    }
    request["parallel_tool_calls"] = json!(false);
    request["tool_choice"] = json!(if accepted { "none" } else { "required" });
    if accepted {
        request["tools"] = json!([]);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle(
    message: &Value,
    thread: &str,
    turn: &str,
    nonce: &str,
    contract: Option<&CodexHarnessClassificationRequest>,
    artifact: Option<&ArtifactCallback>,
    accepted: &mut Option<String>,
    waiting: &mut Option<WorkInputRequest>,
    send: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<bool, String> {
    if message["method"] != "item/tool/call" || message["params"]["tool"] != RESULT_TOOL {
        return Ok(false);
    }
    let id = message
        .get("id")
        .ok_or("workbench_result_request_id_missing")?;
    let params = &message["params"];
    let submit = || -> Result<(String, String, Option<WorkInputRequest>), String> {
        if params["threadId"] != thread
            || params["turnId"] != turn
            || !params["namespace"].is_null()
            || nonce.is_empty()
        {
            return Err("结果不属于当前执行轮次。".into());
        }
        let call_id = params["callId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("缺少原生调用编号。")?;
        crate::app_bridge::validate_tool_arguments(
            &schema(contract.is_some()),
            &params["arguments"],
        )?;
        let value = super::bind_arguments(&params["arguments"], nonce)?;
        let output = value.to_string();
        if output.len() > 1_000_000
            || accepted
                .as_ref()
                .is_some_and(|previous| previous != &output)
        {
            return Err("本轮结果过大或与已提交内容冲突。".into());
        }
        let input: Option<WorkInputRequest> =
            serde_json::from_value(value[FIELD].clone()).map_err(|_| "请明确提交缺项状态。")?;
        if let Some(input) = &input
            && [&input.question, &input.impact].iter().any(|text| {
                text.trim().is_empty()
                    || text.chars().count() > 500
                    || text.chars().any(char::is_control)
            })
        {
            return Err("请用简短文本说明缺项和影响。".into());
        }
        if let Some(contract) = contract {
            crate::app_bridge::validate_tool_arguments(
                &with_execution(contract.output_schema.clone()),
                &value,
            )?;
            (contract.validate)(&without_execution(&output)?)?;
        }
        let paths = value[ARTIFACTS]
            .as_array()
            .ok_or("请明确提交交付文件列表。")?;
        if paths.iter().any(|path| {
            path.as_str()
                .is_none_or(|path| path.trim().is_empty() || path.chars().any(char::is_control))
        }) {
            return Err("交付文件路径不能为空或包含控制字符。".into());
        }
        if accepted.is_none() && !paths.is_empty() {
            let register = artifact.ok_or("当前运行未启用产物登记。")?;
            for path in paths {
                register(path.as_str().ok_or("交付文件路径无效。")?)
                    .map_err(|error| format!("产物登记失败：{error}"))?;
            }
        }
        Ok((output, super::receipt(call_id, &value), input))
    };
    let (success, text) = match submit() {
        Ok((output, receipt, input)) => {
            *accepted = Some(output);
            *waiting = input;
            (true, receipt)
        }
        Err(error) => (false, error),
    };
    send(
        &json!({"id":id,"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}}),
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn submission(arguments: &Value) -> Value {
        json!({"id":1,"method":"item/tool/call","params":{
            "tool":RESULT_TOOL,"threadId":"thread","turnId":"turn","callId":"call-1","arguments":arguments
        }})
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn execution_result_requires_an_explicit_input_decision_and_preserves_its_history_receipt() {
        let arguments = json!({"schemaVersion":1,"reply":"还需要确认新的活动日期，确定后继续原任务。",ARTIFACTS:[],FIELD:{"question":"松风夜读会改到哪一天？","impact":"未确定新日期，改期通知尚不能定稿。"}});
        let mut message = submission(&arguments);
        let mut accepted = None;
        let mut waiting = None;
        let mut response = Value::Null;
        message["params"]["turnId"] = json!("old");
        handle(
            &message,
            "thread",
            "turn",
            "nonce",
            None,
            None,
            &mut accepted,
            &mut waiting,
            &mut |value| {
                response = value.clone();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(response["result"]["success"], false);
        assert!(waiting.is_none());
        message["params"]["turnId"] = json!("turn");
        message["params"]["arguments"]
            .as_object_mut()
            .unwrap()
            .remove(FIELD);
        handle(
            &message,
            "thread",
            "turn",
            "nonce",
            None,
            None,
            &mut accepted,
            &mut waiting,
            &mut |value| {
                response = value.clone();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(response["result"]["success"], false);
        assert!(accepted.is_none());
        message["params"]["arguments"] = arguments.clone();
        handle(
            &message,
            "thread",
            "turn",
            "nonce",
            None,
            None,
            &mut accepted,
            &mut waiting,
            &mut |value| {
                response = value.clone();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(response["result"]["success"], true);
        assert_eq!(waiting.as_ref().unwrap().question, "松风夜读会改到哪一天？");
        let saved = json!({"items":[{"id":"call-1","type":"dynamicToolCall","tool":RESULT_TOOL,"status":"completed","success":true,"arguments":arguments,"contentItems":response["result"]["contentItems"]}]});
        assert_eq!(
            super::super::from_native_turn(&saved, "nonce", true).unwrap(),
            accepted.clone().unwrap()
        );
        assert_eq!(
            super::super::reply(accepted.as_ref().unwrap()).unwrap(),
            "还需要确认新的活动日期，确定后继续原任务。"
        );
        message["params"]["arguments"][FIELD] = Value::Null;
        handle(
            &message,
            "thread",
            "turn",
            "nonce",
            None,
            None,
            &mut accepted,
            &mut waiting,
            &mut |value| {
                response = value.clone();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(response["result"]["success"], false);
        assert!(waiting.is_some());
        accepted = None;
        waiting = None;
        handle(
            &message,
            "thread",
            "turn",
            "nonce-2",
            None,
            None,
            &mut accepted,
            &mut waiting,
            &mut |value| {
                response = value.clone();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(response["result"]["success"], true);
        assert!(waiting.is_none());
    }

    #[test]
    fn execution_contract_is_current_and_stops_tools_only_after_the_formal_receipt() {
        let scope = "run_current";
        let key = context_key(scope);
        let mut request = json!({"input":[
            {"role":"user","content":[{"type":"input_text","text":"当前工作"}]},
            {"role":"developer","content":[{"type":"input_text","text":format!("<{key}>当前规则</{key}>")}]}
        ],"tools":[super::super::tool(),{"type":"function","name":"execute"}],"tool_choice":"auto"});
        assert!(declared(&request, scope));
        assert!(!declared(&request, "run_old"));
        project(&mut request, false).unwrap();
        assert_eq!(request["tool_choice"], "required");
        assert_eq!(
            request["tools"][0]["parameters"]["required"],
            json!(["schemaVersion", "reply", FIELD, ARTIFACTS])
        );
        assert_eq!(request["tools"][1]["name"], "execute");
        request["input"].as_array_mut().unwrap().extend([
            json!({"type":"function_call","name":RESULT_TOOL,"call_id":"done","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"done","output":super::super::ACCEPTED}),
        ]);
        project(&mut request, false).unwrap();
        assert_eq!(request["tool_choice"], "none");
        assert_eq!(request["tools"], json!([]));
        request["input"][1]["role"] = json!("user");
        assert!(!declared(&request, scope));
    }

    #[test]
    fn execution_state_shares_the_classified_result_without_changing_its_validator() {
        let mut output_schema = super::super::model_schema();
        output_schema["properties"]["nonce"] = json!({"type":"string","enum":["nonce"]});
        output_schema["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("nonce"));
        let contract = CodexHarnessClassificationRequest {
            developer_instructions: String::new(),
            output_schema,
            validate: Arc::new(|raw| {
                let value: Value = serde_json::from_str(raw).unwrap();
                assert!(value.get(FIELD).is_none());
                assert!(value.get(ARTIFACTS).is_none());
                Ok(())
            }),
        };
        let args = json!({"schemaVersion":1,"reply":"草稿已完成。","decision":"unclassified","taskId":"","taskTitle":"","objective":"","expectedOutput":"","completionCriteria":"","conversationStatus":"","stage":"","outcomeTitle":"","outcomeSummary":"","reason":"本轮为独立预览。","confidence":1.0,ARTIFACTS:[],FIELD: null});
        let mut accepted = None;
        let mut waiting = None;
        let mut response = Value::Null;
        handle(
            &submission(&args),
            "thread",
            "turn",
            "nonce",
            Some(&contract),
            None,
            &mut accepted,
            &mut waiting,
            &mut |value| {
                response = value.clone();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(response["result"]["success"], true);
        let saved = json!({"items":[{"id":"call-1","type":"dynamicToolCall","tool":RESULT_TOOL,"status":"completed","success":true,"arguments":args,"contentItems":response["result"]["contentItems"]}]});
        assert_eq!(
            super::super::from_native_turn(&saved, "nonce", true).unwrap(),
            accepted.clone().unwrap()
        );
        assert!(
            serde_json::from_str::<Value>(&without_execution(accepted.as_ref().unwrap()).unwrap())
                .unwrap()
                .get(FIELD)
                .is_none()
        );
    }
    #[test]
    fn delivery_requires_file_registration_and_repeated_receipts_do_not_register_again() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let available = Arc::new(AtomicBool::new(false));
        let registrations = Arc::new(AtomicUsize::new(0));
        let callback: ArtifactCallback = {
            let available = Arc::clone(&available);
            let registrations = Arc::clone(&registrations);
            Arc::new(move |path| {
                assert_eq!(path, "通知.md");
                registrations.fetch_add(1, Ordering::SeqCst);
                if available.load(Ordering::SeqCst) {
                    Ok(path.into())
                } else {
                    Err("ai_artifact_unavailable".into())
                }
            })
        };
        let mut args =
            json!({"schemaVersion":1,"reply":"通知已保存。",FIELD:null,ARTIFACTS:["通知.md"]});
        let mut accepted = None;
        let mut waiting = None;
        let mut response = Value::Null;
        let mut submit =
            |args: &Value, callback: Option<&ArtifactCallback>, accepted: &mut Option<String>| {
                handle(
                    &submission(args),
                    "thread",
                    "turn",
                    "nonce",
                    None,
                    callback,
                    accepted,
                    &mut waiting,
                    &mut |value| {
                        response = value.clone();
                        Ok(())
                    },
                )
                .unwrap();
                response.clone()
            };
        assert_eq!(
            submit(&args, None, &mut accepted)["result"]["success"],
            false
        );
        assert!(accepted.is_none());
        let failed = submit(&args, Some(&callback), &mut accepted);
        assert_eq!(failed["result"]["success"], false);
        assert!(failed.to_string().contains("ai_artifact_unavailable"));
        assert!(accepted.is_none());
        available.store(true, Ordering::SeqCst);
        let receipt = submit(&args, Some(&callback), &mut accepted);
        assert_eq!(receipt["result"]["success"], true);
        assert_eq!(submit(&args, Some(&callback), &mut accepted), receipt);
        assert_eq!(registrations.load(Ordering::SeqCst), 2);
        let history = json!({"items":[{"id":"call-1","type":"dynamicToolCall","tool":RESULT_TOOL,"status":"completed","success":true,"arguments":args,"contentItems":receipt["result"]["contentItems"]}]});
        assert_eq!(
            super::super::from_native_turn(&history, "nonce", true).unwrap(),
            accepted.clone().unwrap()
        );
        args[ARTIFACTS] = json!(["另一份.md"]);
        assert_eq!(
            submit(&args, Some(&callback), &mut accepted)["result"]["success"],
            false
        );
        assert_eq!(registrations.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn invalid_or_unscoped_delivery_cannot_register_a_file() {
        let callback: ArtifactCallback = Arc::new(|_| panic!("invalid receipt must not register"));
        let base = json!({"schemaVersion":1,"reply":"答复",FIELD:null,ARTIFACTS:[]});
        let mut cases = vec![submission(&base)];
        cases[0]["params"]["turnId"] = json!("old");
        for paths in [
            Value::Null,
            json!([""]),
            json!(["  "]),
            json!(["a\nb"]),
            json!(["a", "a"]),
        ] {
            let mut args = base.clone();
            args[ARTIFACTS] = paths;
            cases.push(submission(&args));
        }
        let mut omitted = base.clone();
        omitted.as_object_mut().unwrap().remove(ARTIFACTS);
        cases.push(submission(&omitted));
        let mut mixed = base.clone();
        mixed[crate::public_reasoning::FIELD] = json!("已核对日期和地点。");
        cases.push(submission(&mixed));
        for message in cases {
            let mut accepted = None;
            let mut response = Value::Null;
            handle(
                &message,
                "thread",
                "turn",
                "nonce",
                None,
                Some(&callback),
                &mut accepted,
                &mut None,
                &mut |value| {
                    response = value.clone();
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(response["result"]["success"], false);
            assert!(accepted.is_none());
        }
        let mut accepted = None;
        handle(
            &submission(&base),
            "thread",
            "turn",
            "nonce",
            None,
            Some(&callback),
            &mut accepted,
            &mut None,
            &mut |_| Ok(()),
        )
        .unwrap();
        assert!(accepted.is_some());
    }
}
