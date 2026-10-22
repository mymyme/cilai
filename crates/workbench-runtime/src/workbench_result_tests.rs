use super::*;
use std::sync::Arc;

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
    let valid =
        json!({"schemaVersion":1,"nonce":"current","reply":"真实结果","decision":"unclassified"});
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
            json!({"schemaVersion":1,"nonce":"current","reply":"缺字段"}),
            false,
        ),
        (
            "thread",
            "turn",
            json!({"schemaVersion":1,"nonce":"current","reply":"错误语义","decision":"create"}),
            false,
        ),
        ("thread", "turn", valid.clone(), true),
        ("thread", "turn", valid.clone(), true),
        (
            "thread",
            "turn",
            json!({"schemaVersion":1,"nonce":"current","reply":"覆盖","decision":"unclassified"}),
            false,
        ),
    ] {
        let message = json!({"id":1,"method":"item/tool/call","params":{"threadId":thread,"turnId":turn,"tool":RESULT_TOOL,"arguments":args}});
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
    assert_eq!(accepted, Some(valid.to_string()));
}

#[test]
fn formal_result_nonce_feedback_allows_same_turn_correction_without_cross_turn_disclosure() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let nonce = "classification_231599bf-e0f6-4974-8396-2d23ee6fe2a3";
    let validated = Arc::new(AtomicUsize::new(0));
    let mut contract = contract();
    contract.output_schema["properties"]["nonce"]["enum"] = json!([nonce]);
    let calls = Arc::clone(&validated);
    contract.validate = Arc::new(move |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let mut accepted = None;
    for (thread, turn, expected_feedback) in [
        ("other", "turn", false),
        ("thread", "other", false),
        ("thread", "turn", true),
    ] {
        let message = json!({"id":1,"method":"item/tool/call","params":{
            "threadId":thread,"turnId":turn,"tool":RESULT_TOOL,
            "arguments":{"schemaVersion":1,"nonce":"invented","reply":"只读核对结果","decision":"unclassified"}}});
        let mut response = Value::Null;
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
        assert_eq!(response["result"]["success"], false);
        let text = response["result"]["contentItems"][0]["text"]
            .as_str()
            .unwrap();
        assert_eq!(text.contains(nonce), expected_feedback);
        if expected_feedback {
            assert!(text.contains("无需重复已完成的业务工具或成果登记"));
            assert!(text.contains("错误结果尚未接收"));
        }
        assert!(accepted.is_none());
        assert_eq!(validated.load(Ordering::SeqCst), 0);
    }
    let valid =
        json!({"schemaVersion":1,"nonce":nonce,"reply":"只读核对结果","decision":"unclassified"});
    let message = json!({"id":2,"method":"item/tool/call","params":{
        "threadId":"thread","turnId":"turn","tool":RESULT_TOOL,"arguments":valid}});
    let mut response = Value::Null;
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
    assert_eq!(accepted, Some(valid.to_string()));
    assert_eq!(validated.load(Ordering::SeqCst), 1);
}

#[test]
fn history_requires_a_successful_current_result_and_rejects_conflicts() {
    let accepted = json!({"id":"tool-1","type":"dynamicToolCall","tool":RESULT_TOOL,"status":"completed","success":true,
        "arguments":{"schemaVersion":1,"nonce":"current","reply":"正式结果","decision":"unclassified"}});
    let mut rejected = accepted.clone();
    rejected["success"] = json!(false);
    rejected["arguments"]["nonce"] = json!("old");
    let prose = json!({"type":"agentMessage","text":"已提交正式结果"});
    assert!(from_native_turn(&json!({"items":[prose,rejected.clone()]}), "current").is_err());
    assert_eq!(
        from_native_turn(
            &json!({"items":[rejected,accepted.clone(),accepted.clone()]}),
            "current"
        )
        .unwrap(),
        accepted["arguments"].to_string()
    );
    assert!(from_native_turn(&json!({"items":[accepted.clone()]}), "other").is_err());
    let mut conflicting = accepted.clone();
    conflicting["arguments"]["reply"] = json!("覆盖");
    assert!(from_native_turn(&json!({"items":[accepted,conflicting]}), "current").is_err());
}
