use super::{CLASSIFICATION_TOOL, ClassificationSubmission};
use serde_json::{Value, json};
use std::sync::Arc;

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["schemaVersion", "nonce", "reply", "decision", "reason"],
        "properties": {
            "schemaVersion": { "type": "integer", "enum": [1] },
            "nonce": { "type": "string", "enum": ["classification-run"] },
            "reply": { "type": "string", "minLength": 1 },
            "decision": { "type": "string", "enum": ["create", "unclassified"] },
            "reason": { "type": "string", "minLength": 1 }
        },
        "allOf": [{ "if": { "properties": { "decision": { "const": "create" } } }, "then": {} }]
    })
}

/// 模拟工作台的完整信封校验：decision=create 时 reason 不得为"无"。
fn submission() -> ClassificationSubmission {
    ClassificationSubmission::new(
        &schema(),
        Arc::new(|envelope| {
            let value: Value = serde_json::from_str(envelope).map_err(|error| error.to_string())?;
            if value["decision"] == "create" && value["reason"] == "无" {
                return Err("create 必须说明理由".into());
            }
            Ok(())
        }),
    )
    .unwrap()
}

fn call(arguments: Value, turn: &str) -> Value {
    let mut message = json!({
        "id": 7,
        "method": "item/tool/call",
        "params": { "threadId": "thread", "turnId": turn, "tool": CLASSIFICATION_TOOL }
    });
    message["params"]["arguments"] = arguments;
    message
}

fn submit(submission: &ClassificationSubmission, message: &Value) -> Value {
    let mut sent = Vec::new();
    assert!(
        submission
            .handle(message, "thread", "turn", &mut |value| {
                sent.push(value.clone());
                Ok(())
            })
            .unwrap()
    );
    sent.pop().unwrap()["result"].clone()
}

#[test]
fn tool_declares_classification_fields_without_reply() {
    let tool = submission().tool();
    assert_eq!(tool["name"], CLASSIFICATION_TOOL);
    let input = &tool["inputSchema"];
    assert!(input["properties"]["reply"].is_null());
    assert_eq!(
        input["properties"]["nonce"]["enum"],
        json!(["classification-run"])
    );
    assert_eq!(
        input["required"],
        json!(["schemaVersion", "nonce", "decision", "reason"])
    );
    assert!(input.get("allOf").is_none());
}

#[test]
fn accepted_classification_is_rebuilt_with_the_final_reply() {
    let submission = submission();
    assert_eq!(submission.envelope("正文").unwrap(), None);
    let result = submit(
        &submission,
        &call(
            json!({"schemaVersion":1,"nonce":"classification-run","decision":"create","reason":"新工作"}),
            "turn",
        ),
    );
    assert_eq!(result["success"], true);
    let envelope: Value =
        serde_json::from_str(&submission.envelope("已完成热力图").unwrap().unwrap()).unwrap();
    assert_eq!(
        envelope,
        json!({"schemaVersion":1,"nonce":"classification-run","decision":"create","reason":"新工作","reply":"已完成热力图"})
    );
}

#[test]
fn rejected_submissions_explain_the_reason_and_keep_the_previous_result() {
    let submission = submission();
    for (arguments, turn) in [
        (
            json!({"schemaVersion":1,"nonce":"classification-old","decision":"create","reason":"x"}),
            "turn",
        ),
        (
            json!({"schemaVersion":1,"nonce":"classification-run","decision":"create","reason":"无"}),
            "turn",
        ),
        (
            json!({"schemaVersion":1,"nonce":"classification-run","decision":"create","reason":"x"}),
            "other-turn",
        ),
    ] {
        let result = submit(&submission, &call(arguments, turn));
        assert_eq!(result["success"], false);
        assert!(
            result["contentItems"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("归类未接收：")
        );
    }
    assert_eq!(submission.envelope("正文").unwrap(), None);
}

#[test]
fn other_tool_calls_are_left_to_their_handlers() {
    let mut message = call(json!({}), "turn");
    message["params"]["tool"] = json!("cilai_register_artifact");
    assert!(
        !submission()
            .handle(&message, "thread", "turn", &mut |_| Ok(()))
            .unwrap()
    );
}
