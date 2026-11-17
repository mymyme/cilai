use super::*;
use std::sync::Arc;

#[test]
fn public_note_and_formal_reply_streams_do_not_mix_their_tools() {
    let event = |name: &str, id: &str, arguments: Value| {
        json!({
            "type":"response.output_item.added","item":{
                "type":"function_call","name":name,"id":id,"call_id":id,
                "arguments":arguments.to_string()
            }
        })
    };
    let note = event(
        crate::public_reasoning::TOOL,
        "note",
        json!({"publicReasoning":"先核对日期和地点。"}),
    );
    let result = event(RESULT_TOOL, "result", json!({"reply":"邀请预览已经完成。"}));
    let mut public = ReplyStream::for_tool(
        crate::public_reasoning::TOOL,
        crate::public_reasoning::FIELD,
    );
    let mut formal = ReplyStream::default();
    public.observe(&result);
    formal.observe(&note);
    assert!(public.take_preview().is_none());
    assert!(formal.take_preview().is_none());
    public.observe(&note);
    formal.observe(&result);
    assert_eq!(public.take_preview().as_deref(), Some("先核对日期和地点。"));
    assert_eq!(formal.take_preview().as_deref(), Some("邀请预览已经完成。"));
    assert!(public.take_preview().is_none());
    assert!(formal.take_preview().is_none());
}

fn contract() -> CodexHarnessClassificationRequest {
    CodexHarnessClassificationRequest {
        developer_instructions: String::new(),
        output_schema: json!({"type":"object","additionalProperties":false,
            "properties":{"schemaVersion":{"type":"integer","enum":[1]},
                "nonce":{"type":"string","enum":["current"]},
                "reply":{"type":"string"},"decision":{"type":"string"}},
            "required":["schemaVersion","nonce","reply","decision"]}),
        validate: Arc::new(|raw| {
            let value: Value = serde_json::from_str(raw).unwrap();
            if value["decision"] != "unclassified" {
                return Err("任务语义不合格".into());
            }
            Ok(())
        }),
    }
}

#[test]
fn formal_result_rejects_scope_schema_semantics_and_conflicting_resubmission() {
    let contract = contract();
    let valid = json!({"schemaVersion":1,"reply":"真实结果","decision":"unclassified"});
    let mut accepted = None;
    for (thread, turn, args, expected) in [
        ("other", "turn", valid.clone(), false),
        ("thread", "other", valid.clone(), false),
        (
            "thread",
            "turn",
            json!({"schemaVersion":1,"nonce":"old","reply":"旧结果","decision":"unclassified"}),
            false,
        ),
        (
            "thread",
            "turn",
            json!({"schemaVersion":1,"reply":"缺字段"}),
            false,
        ),
        (
            "thread",
            "turn",
            json!({"schemaVersion":1,"reply":"错误语义","decision":"create"}),
            false,
        ),
        ("thread", "turn", valid.clone(), true),
        ("thread", "turn", valid.clone(), true),
        (
            "thread",
            "turn",
            json!({"schemaVersion":1,"reply":"覆盖","decision":"unclassified"}),
            false,
        ),
    ] {
        let message = json!({"id":1,"method":"item/tool/call","params":{"threadId":thread,"turnId":turn,"callId":"call-1","tool":RESULT_TOOL,"arguments":args}});
        let mut response = Value::Null;
        assert!(
            handle(
                &message,
                "thread",
                "turn",
                Some(&contract),
                &mut accepted,
                &mut |value| {
                    response = value.clone();
                    Ok(())
                }
            )
            .unwrap()
        );
        assert_eq!(response["result"]["success"], expected);
    }
    assert_eq!(
        accepted,
        Some(bind_arguments(&valid, "current").unwrap().to_string())
    );
}

#[test]
fn host_binds_nonce_and_recovery_checks_the_call_receipt() {
    let contract = contract();
    let arguments = json!({"schemaVersion":1,"reply":"正式结果","decision":"unclassified"});
    let mut response = Value::Null;
    let mut accepted = None;
    let message = json!({"id":2,"method":"item/tool/call","params":{
        "threadId":"thread","turnId":"turn","callId":"call-1","tool":RESULT_TOOL,"arguments":arguments}});
    handle(
        &message,
        "thread",
        "turn",
        Some(&contract),
        &mut accepted,
        &mut |value| {
            response = value.clone();
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(response["result"]["success"], true);
    assert!(
        !response["result"]["contentItems"][0]["text"]
            .as_str()
            .unwrap()
            .contains("current")
    );
    let mut item = json!({"type":"dynamicToolCall","tool":RESULT_TOOL,"status":"completed","success":true,
        "id":"call-1","arguments":arguments,"contentItems":response["result"]["contentItems"]});
    let read = |item| from_native_turn(&json!({"items":[item]}), "current", true);
    assert_eq!(read(item.clone()).unwrap(), accepted.unwrap());
    assert!(from_native_turn(&json!({"items":[item.clone()]}), "old", true).is_err());
    item["id"] = json!("replayed");
    assert!(read(item.clone()).is_err());
    item["id"] = json!("call-1");
    item["arguments"]["reply"] = json!("伪造正文");
    assert!(read(item).is_err());
}

#[test]
fn history_requires_a_successful_current_result_and_rejects_conflicts() {
    let accepted = json!({"id":"tool-1","type":"dynamicToolCall","tool":RESULT_TOOL,"status":"completed","success":true,
        "arguments":{"schemaVersion":1,"nonce":"current","reply":"正式结果","decision":"unclassified"}});
    let mut rejected = accepted.clone();
    rejected["success"] = json!(false);
    rejected["arguments"]["nonce"] = json!("old");
    let prose = json!({"type":"agentMessage","text":"已提交正式结果"});
    assert!(
        from_native_turn(&json!({"items":[prose,rejected.clone()]}), "current", false).is_err()
    );
    assert_eq!(
        from_native_turn(
            &json!({"items":[rejected,accepted.clone(),accepted.clone()]}),
            "current",
            false
        )
        .unwrap(),
        accepted["arguments"].to_string()
    );
    assert!(from_native_turn(&json!({"items":[accepted.clone()]}), "other", false).is_err());
    let mut conflicting = accepted.clone();
    conflicting["arguments"]["reply"] = json!("覆盖");
    assert!(from_native_turn(&json!({"items":[accepted,conflicting]}), "current", false).is_err());
}

#[test]
fn thread_declaration_is_stable_without_freezing_a_run_nonce_or_task_decision() {
    let first = tool();
    let second = tool();
    assert_eq!(first, second);
    assert_eq!(
        first["inputSchema"]["required"].as_array().unwrap().len(),
        14
    );
    assert!(first["inputSchema"]["properties"].get("nonce").is_none());
    assert_eq!(first["inputSchema"]["additionalProperties"], false);
    for decision in ["keep", "create", "link", "unclassified"] {
        assert!(
            first["inputSchema"]["properties"]["decision"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!(decision))
        );
    }
}

#[test]
fn visible_thread_tool_without_current_result_authority_cannot_submit() {
    let message = json!({"id":1,"method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","tool":RESULT_TOOL,"arguments":{}}});
    let mut accepted = None;
    let mut response = Value::Null;
    handle(
        &message,
        "thread",
        "turn",
        None,
        &mut accepted,
        &mut |value| {
            response = value.clone();
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(response["result"]["success"], false);
    assert!(accepted.is_none());
}

#[test]
fn model_history_keeps_only_current_control_context_and_keeps_business_history() {
    let current = "run_aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let old = "run_bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let text = |key: &str, body: &str| json!({"type":"input_text","text":format!("<{key}>{body}</{key}>")});
    let developer = |parts| json!({"type":"message","role":"developer","content":parts});
    let current_parts = vec![
        text(&contract_key(current), "当轮"),
        text(
            &format!("{}_part_000001", contract_key(current)),
            "当轮片段",
        ),
        text("cilai_capture_response_contract", "采集约定"),
    ];
    let unrelated = text("cilai_turn_materials", "RTO 历史证据");
    let user = json!({"type":"message","role":"user","content":[text("cilai_response_contract","员工原文")]});
    let mut body = json!({"input":[
        developer(vec![text("cilai_response_contract", "旧约定")]),
        developer(vec![text("cilai_response_contract_part_000001", "旧片段"), unrelated.clone()]),
        developer(vec![text(&contract_key(old), "先前运行"), text(&format!("{}_part_000001", contract_key(old)), "先前片段")]),
        user,
        developer(current_parts.clone()),
        {"type":"function_call","name":RESULT_TOOL,"call_id":"result-call","arguments":r#"{"nonce":"old","reply":"历史结果"}"#},
        {"type":"function_call_output","call_id":"result-call","output":"当前本轮 nonce 的唯一允许值是 old"},
        {"type":"function_call_output","call_id":"rto-query","output":"历史查询回执"}
    ]});
    assert!(project_model_history(&mut body, current));
    assert_eq!(
        body["input"],
        json!([
            developer(vec![unrelated]),
            user,
            developer(current_parts),
            {"type":"function_call","name":RESULT_TOOL,"call_id":"result-call","arguments":r#"{"reply":"历史结果"}"#},
            {"type":"function_call_output","call_id":"result-call","output":"该轮正式结果提交曾被拒绝。"},
            {"type":"function_call_output","call_id":"rto-query","output":"历史查询回执"}
        ])
    );
    assert!(!project_model_history(&mut body, current));
}

#[test]
fn only_the_current_formal_receipt_stops_reply_previews() {
    let user = json!({"type":"message","role":"user"});
    let call = json!({"type":"function_call","name":RESULT_TOOL,"call_id":"result"});
    let other = json!({"type":"function_call_output","call_id":"other","output":ACCEPTED});
    let result = json!({"type":"function_call_output","call_id":"result","output":ACCEPTED});
    assert!(!reply_already_accepted(&[
        user.clone(),
        call.clone(),
        other
    ]));
    assert!(reply_already_accepted(&[
        user.clone(),
        call.clone(),
        result.clone()
    ]));
    assert!(!reply_already_accepted(&[call, result, user]));
}
