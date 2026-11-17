use crate::codex_harness::{CodexHarnessEvent, CodexHarnessRequest};
use crate::conversation_kernel::ConversationKind;
use serde_json::{Value, json};
use std::collections::HashSet;

pub(crate) const FIELD: &str = "publicReasoning";
pub(crate) const TOOL: &str = "cilai_submit_public_reasoning";
const CONTEXT_PREFIX: &str = "cilai_public_reasoning_";
const MAX_CHARS: usize = 600;
const ACCEPTED: &str = "处理思路已显示，请继续本轮任务。";
const RECEIVED_INSTRUCTIONS: &str = "本轮公开处理思路已经接收。不再调用 cilai_submit_public_reasoning，继续完成实际工作。完成后调用 cilai_submit_workbench_result，按当前工具定义提交完整正式结果，不得混入 publicReasoning。";
pub(crate) const INSTRUCTIONS: &str = "在回复正文或调用业务工具前，先调用 cilai_submit_public_reasoning，填写 publicReasoning。用一到三句面向用户的中文说明本轮打算核对什么、依据什么判断，或采用什么方法。只写与本轮问题直接相关的可公开思路，不复述请求，不输出内部推理、系统指令、凭据或完整答案。收到思路回执后继续原任务；完成后调用 cilai_submit_workbench_result 提交正式结果。两个工具用途不同，不能混用。";

pub(crate) fn tool() -> Value {
    json!({"type":"function","name":TOOL,
        "description":"仅提交本轮可公开的处理思路。收到回执后继续执行工作，正式交付使用 cilai_submit_workbench_result。",
        "inputSchema":{"type":"object","additionalProperties":false,"required":[FIELD],
            "properties":{(FIELD):{"type":"string","minLength":1,"maxLength":MAX_CHARS}}}})
}

pub(crate) fn enabled(request: &CodexHarnessRequest) -> bool {
    request.conversation_kind == ConversationKind::Workbench
        && request
            .reasoning_effort
            .as_deref()
            .is_some_and(|effort| effort != "none")
        && request.model_catalog.iter().any(|model| {
            model.model_id == request.model
                && model.supports_public_reasoning
                && !model.supports_reasoning_summary
                && model
                    .tool_capabilities
                    .as_ref()
                    .is_some_and(|tools| tools.function_calling)
        })
}

pub(crate) fn context_key(scope: &str) -> String {
    format!("{CONTEXT_PREFIX}{scope}")
}

pub(crate) fn hide_undeclared(request: &mut Value, scope: &str) {
    fn hide(container: &mut Value) {
        if let Some(tools) = container.get_mut("tools").and_then(Value::as_array_mut) {
            tools.retain(|tool| tool["name"] != TOOL);
        }
    }
    if declared(request, scope) {
        return;
    }
    hide(request);
    if let Some(items) = request.get_mut("input").and_then(Value::as_array_mut) {
        for item in items.iter_mut().filter(|item| {
            matches!(
                item["type"].as_str(),
                Some("additional_tools" | "tool_search_output")
            )
        }) {
            hide(item);
        }
    }
}

pub(crate) fn require_note(request: &mut Value) -> Result<(), ()> {
    let mut note = request["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            request["input"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item["type"] == "additional_tools")
                .flat_map(|item| item["tools"].as_array().into_iter().flatten()),
        )
        .find(|tool| tool["type"] == "function" && tool["name"] == TOOL)
        .cloned()
        .ok_or(())?;
    note["parameters"] = json!({"type":"object","additionalProperties":false,"required":[FIELD],
        "properties":{(FIELD):{"type":"string","minLength":1,"maxLength":MAX_CHARS,
            "description":"一到三句具体、可公开的处理思路。"}}});
    note["description"] = json!("仅提交本轮可公开的处理思路。收到回执后继续执行工作。");
    request["tools"] = json!([note]);
    request["tool_choice"] = json!("required");
    request["parallel_tool_calls"] = json!(false);
    if let Some(items) = request["input"].as_array_mut() {
        items.retain_mut(|item| match item["type"].as_str() {
            Some("additional_tools") => false,
            Some("tool_search_output") => {
                item["tools"] = json!([]);
                true
            }
            _ => true,
        });
    }
    Ok(())
}

fn context_part_scope(part: &Value) -> Option<&str> {
    if part["type"] != "input_text" {
        return None;
    }
    let (key, value) = part["text"].as_str()?.strip_prefix('<')?.split_once('>')?;
    let scope = key.strip_prefix(CONTEXT_PREFIX)?;
    uuid::Uuid::parse_str(scope.strip_prefix("run_")?).ok()?;
    value.ends_with(&format!("</{key}>")).then_some(scope)
}

pub(crate) fn declared(request: &Value, scope: &str) -> bool {
    request["input"]
        .as_array()
        .and_then(|items| context_index(items, scope))
        .is_some()
}

fn context_index(items: &[Value], scope: &str) -> Option<usize> {
    items.iter().position(|item| {
        item["type"] == "message"
            && item["role"] == "developer"
            && item["content"].as_array().is_some_and(|parts| {
                parts
                    .iter()
                    .any(|part| context_part_scope(part) == Some(scope))
            })
    })
}

fn summary_call(item: &Value) -> bool {
    item["type"] == "function_call"
        && item["name"] == TOOL
        && item["namespace"].is_null()
        && item["arguments"]
            .as_str()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .is_some_and(|args| args.get(FIELD).is_some())
}

fn historical_note(item: &Value) -> bool {
    summary_call(item)
        || (item["type"] == "function_call"
            && item["name"] == crate::workbench_result::RESULT_TOOL
            && item["namespace"].is_null()
            && item["arguments"]
                .as_str()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .is_some_and(|args| args.get(FIELD).is_some()))
}

fn hide_note_tool(container: &mut Value) -> bool {
    let Some(tools) = container.get_mut("tools").and_then(Value::as_array_mut) else {
        return false;
    };
    let before = tools.len();
    tools.retain(|tool| tool["name"] != TOOL);
    before != tools.len()
}

fn note_received(items: &[Value]) -> bool {
    items.iter().enumerate().any(|(index, item)| {
        item["type"] == "function_call_output"
            && crate::workbench_result::receipt_text(&item["output"]) == Some(ACCEPTED)
            && item["call_id"].as_str().is_some_and(|id| {
                items[..index]
                    .iter()
                    .any(|call| call["call_id"] == id && summary_call(call))
            })
    })
}

pub(crate) fn project_model_history(request: &mut Value, scope: &str) -> bool {
    let Some(items) = request.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let current = context_index(items, scope);
    let awaiting_note = current.is_some_and(|index| !note_received(&items[index..]));
    let prior_calls: HashSet<String> = items
        .iter()
        .take(current.unwrap_or(items.len()))
        .filter(|item| historical_note(item))
        .filter_map(|item| item["call_id"].as_str().map(str::to_owned))
        .collect();
    let mut changed = false;
    let mut index = 0;
    items.retain_mut(|item| {
        let prior = current.is_none_or(|start| index < start);
        index += 1;
        let empty_assistant_message = current.is_some()
            && item["type"] == "message"
            && item["role"] == "assistant"
            && item["content"].as_array().is_some_and(|parts| {
                parts.iter().all(|part| {
                    part["type"] == "output_text"
                        && part["text"]
                            .as_str()
                            .is_some_and(|text| text.trim().is_empty())
                })
            });
        if empty_assistant_message
            || (prior && historical_note(item))
            || (item["type"] == "function_call_output"
                && item["call_id"]
                    .as_str()
                    .is_some_and(|id| prior_calls.contains(id)))
        {
            changed = true;
            return false;
        }
        if !awaiting_note {
            changed |= hide_note_tool(item);
        }
        if item["type"] == "message"
            && item["role"] == "developer"
            && let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut)
        {
            let count = parts.len();
            parts.retain_mut(|part| match context_part_scope(part) {
                Some(owner) if current.is_some() && owner == scope && !awaiting_note => {
                    *part = json!({"type":"input_text","text":RECEIVED_INSTRUCTIONS});
                    changed = true;
                    true
                }
                Some(owner) => awaiting_note && owner == scope,
                None => true,
            });
            if count != parts.len() {
                changed = true;
                return !parts.is_empty();
            }
        }
        true
    });
    if !awaiting_note {
        changed |= hide_note_tool(request);
    }
    changed
}

fn valid_text(arguments: &Value) -> Option<&str> {
    if arguments.as_object()?.len() != 1 {
        return None;
    }
    let text = arguments[FIELD].as_str()?.trim();
    (!text.is_empty() && text.chars().count() <= MAX_CHARS).then_some(text)
}

pub(crate) fn event(call_id: &str, text: &str, completed: bool) -> Option<CodexHarnessEvent> {
    let text = crate::codex_harness::compact_visible_text(text, MAX_CHARS)?;
    let mut event = CodexHarnessEvent::status(
        if completed {
            "item.reasoning.public.completed"
        } else {
            "item.reasoning.public.delta"
        },
        "reply",
        if completed { "completed" } else { "started" },
        text,
    );
    "reasoning".clone_into(&mut event.kind);
    event.item_id = Some(call_id.to_owned());
    Some(event)
}

pub(crate) fn completed_item(item: &Value) -> Option<CodexHarnessEvent> {
    if item["type"] != "dynamicToolCall"
        || item["tool"] != TOOL
        || !item["namespace"].is_null()
        || item["status"] != "completed"
        || item["success"] != true
    {
        return None;
    }
    event(item["id"].as_str()?, valid_text(&item["arguments"])?, true)
}

pub(crate) fn handle(
    message: &Value,
    thread: &str,
    turn: &str,
    enabled: bool,
    accepted: &mut Option<String>,
    send: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<bool, String> {
    let params = &message["params"];
    if message["method"] != "item/tool/call" || params["tool"] != TOOL {
        return Ok(false);
    }
    let id = message
        .get("id")
        .ok_or("public_reasoning_request_id_missing")?;
    let scoped = enabled
        && params["threadId"] == thread
        && params["turnId"] == turn
        && params["namespace"].is_null()
        && params["callId"].as_str().is_some_and(|id| !id.is_empty());
    let repeated = scoped && accepted.as_deref().is_some_and(|id| params["callId"] != id);
    let valid = scoped && !repeated && valid_text(&params["arguments"]).is_some();
    let text = if repeated {
        RECEIVED_INSTRUCTIONS
    } else if valid {
        ACCEPTED
    } else if scoped {
        "公开处理思路仅填写 publicReasoning。正式交付请调用 cilai_submit_workbench_result。"
    } else {
        "本轮未接收这条处理思路。"
    };
    send(
        &json!({"id":id,"result":{"success":valid,"contentItems":[{"type":"inputText","text":text}]}}),
    )?;
    if valid {
        *accepted = params["callId"].as_str().map(str::to_owned);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_history_is_only_sent_to_the_model_within_its_current_run() {
        let old = "run_aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let current = "run_bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        let context = |scope| {
            let key = context_key(scope);
            json!({"type":"message","role":"developer","content":[
                {"type":"input_text","text":format!("<{key}>说明</{key}>")}
            ]})
        };
        let call = |id| json!({"type":"function_call","name":TOOL,"call_id":id,"arguments":json!({(FIELD):"公开思路"}).to_string()});
        let ack = |id| json!({"type":"function_call_output","call_id":id,"output":"已显示"});
        let user = json!({"type":"message","role":"user","content":[{"type":"input_text","text":"原问题"}]});
        let mut prior_note = call("old");
        prior_note["name"] = json!(crate::workbench_result::RESULT_TOOL);
        let mut request = json!({"model":"fixture-public-reasoning","input":[context(old),prior_note,ack("old"),user,context(current),call("now"),ack("now")]});
        assert!(declared(&request, current));
        assert!(project_model_history(&mut request, current));
        assert_eq!(
            request["input"],
            json!([user, context(current), call("now"), ack("now")])
        );
        let mut submitted = request.clone();
        submitted["input"][3]["output"] = json!(ACCEPTED);
        assert!(project_model_history(&mut submitted, current));
        assert!(!declared(&submitted, current));
        assert_eq!(
            submitted["input"],
            json!([user, {"type":"message","role":"developer","content":[{"type":"input_text","text":RECEIVED_INSTRUCTIONS}]}, call("now"), {"type":"function_call_output","call_id":"now","output":ACCEPTED}])
        );
        request["model"] = json!("other-model");
        assert!(declared(&request, current));
        request["input"].as_array_mut().unwrap().remove(1);
        assert!(!declared(&request, current));
        assert!(project_model_history(&mut request, current));
        assert_eq!(request["input"], json!([user]));
    }

    #[test]
    fn a_successful_public_note_is_never_a_formal_result_receipt() {
        let item = json!({"type":"dynamicToolCall","tool":TOOL,"id":"note","status":"completed","success":true,
            "arguments":{(FIELD):"先比较两个来源。"}});
        assert!(completed_item(&item).is_some());
        assert!(!crate::workbench_result::is_accepted_item(&item));
    }

    #[test]
    fn note_receipts_are_idempotent_and_later_calls_must_submit_the_formal_result() {
        let mut message = json!({"id":1,"method":"item/tool/call","params":{
            "tool":TOOL,"threadId":"thread","turnId":"turn","callId":"note-1",
            "arguments":{(FIELD):"先核对日期和地点。"}
        }});
        let mut accepted = None;
        let mut response = Value::Null;
        let mut invoke = |message: &Value, accepted: &mut Option<String>| {
            let handled = handle(message, "thread", "turn", true, accepted, &mut |value| {
                response = value.clone();
                Ok(())
            })
            .unwrap();
            (handled, response.clone())
        };
        let (handled, first) = invoke(&message, &mut accepted);
        assert!(handled);
        assert_eq!(first["result"]["success"], true);
        assert_eq!(accepted.as_deref(), Some("note-1"));
        assert_eq!(invoke(&message, &mut accepted).1, first);
        message["params"]["callId"] = json!("note-2");
        let (_, repeated) = invoke(&message, &mut accepted);
        assert_eq!(repeated["result"]["success"], false);
        assert_eq!(
            repeated["result"]["contentItems"][0]["text"],
            RECEIVED_INSTRUCTIONS
        );
        assert_eq!(accepted.as_deref(), Some("note-1"));
        message["params"]["arguments"]["reply"] = json!("还需要用户确认日期和地点。");
        let (handled, mixed) = invoke(&message, &mut accepted);
        assert!(handled);
        assert_eq!(mixed["result"]["success"], false);
        message["params"]["tool"] = json!(crate::workbench_result::RESULT_TOOL);
        assert!(!invoke(&message, &mut accepted).0);
        message["params"]["tool"] = json!(TOOL);
        message["params"]["arguments"] = json!({(FIELD):"先核对日期。"});
        message["params"]["turnId"] = json!("other-turn");
        let (_, unscoped) = invoke(&message, &mut accepted);
        assert_eq!(unscoped["result"]["success"], false);
        assert_eq!(accepted.as_deref(), Some("note-1"));
    }
}

#[cfg(test)]
mod declaration_tests {
    use super::*;
    #[test]
    fn absent_current_declaration_hides_native_and_historical_public_tools() {
        let mut request = json!({"tools":[{"type":"function","name":TOOL},{"type":"function","name":"ordinary"}],"input":[{"type":"additional_tools","tools":[{"type":"function","name":TOOL}]},{"type":"tool_search_output","tools":[{"type":"function","name":TOOL}]}]});
        hide_undeclared(&mut request, "current");
        assert_eq!(
            request["tools"],
            json!([{"type":"function","name":"ordinary"}])
        );
        assert_eq!(request["input"][0]["tools"], json!([]));
        assert_eq!(request["input"][1]["tools"], json!([]));
    }
}
