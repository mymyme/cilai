use super::native_history::hydrate;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn fixture() -> (
    Value,
    Value,
    BTreeMap<String, Value>,
    BTreeMap<String, Vec<Value>>,
) {
    let state = json!({"conversations":[{"conversationId":"c"}],"messages":[{"messageId":"m","conversationId":"c","runId":"r","role":"user","contentDigest":super::sha256_hex("原文"),"createdAt":1000}],"runs":[{"runId":"r","conversationId":"c","status":"running","startedAt":1000,"classificationStatus":"pending"}]});
    let ledger = json!({"outbox":[],"messageRefs":[],"runRefs":[{"runId":"r","nativeThreadId":"thread","nativeTurnId":"turn"}],"native":[{"conversationId":"c","journal":{"binding":{"nativeThreadId":"thread","generation":1},"messages":[{"messageId":"m","nativeTurnId":"turn","clientMessageId":"client-m"},{"messageId":"steer","nativeTurnId":"turn","clientMessageId":"client-steer"}],"operations":[{"generation":1,"state":"acknowledged","request":{"kind":"steer","runId":"r","input":{"messageId":"steer","createdAt":2000}},"receipt":{"nativeThreadId":"thread","nativeTurnId":"turn","clientMessageId":"client-steer"}}]}}]});
    let catalog = BTreeMap::from([("thread".into(), json!({"name":"原生名称","archived":true}))]);
    let turns = BTreeMap::from([(
        "thread".into(),
        vec![
            json!({"id":"turn","status":"completed","completedAt":3,"items":[{"type":"userMessage","id":"u","clientId":"client-m","content":[{"type":"text","text":"原文"}]},{"type":"userMessage","id":"s","clientId":"client-steer","content":[{"type":"text","text":"补充"}]},{"type":"agentMessage","id":"a","phase":"final_answer","text":"完成回复"}]}),
        ],
    )]);
    (state, ledger, catalog, turns)
}
#[test]
fn recovers_acknowledged_input_and_completed_reply_without_using_diagnostic_logs() {
    let (mut state, ledger, catalog, turns) = fixture();
    hydrate(&mut state, &ledger, &catalog, &turns).unwrap();
    assert_eq!(
        state["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["content"].clone())
            .collect::<Vec<_>>(),
        vec![json!("原文"), json!("补充"), json!("完成回复")]
    );
    assert_eq!(state["runs"][0]["status"], "completed");
    assert_eq!(state["runs"][0]["output"], "完成回复");
    assert_eq!(state["runs"][0]["classificationStatus"], "failed");
    assert_eq!(state["conversations"][0]["title"], "原生名称");
    assert_eq!(state["conversations"][0]["archived"], true);
}
#[test]
fn native_body_digest_mismatch_is_an_explicit_error() {
    let (mut state, ledger, catalog, mut turns) = fixture();
    turns.get_mut("thread").unwrap()[0]["items"][0]["content"][0]["text"] = json!("已改变");
    assert_eq!(
        hydrate(&mut state, &ledger, &catalog, &turns).unwrap_err(),
        "native_history_content_mismatch:m"
    );
}

#[test]
fn restart_normalizes_native_reply_boundaries_like_live_completion() {
    let body = "已记住青竹731\n\n  正文内部缩进";
    for output in [
        format!("\n\n{body}\n "),
        json!({"reply":format!("\n{body}\n")}).to_string(),
    ] {
        let (mut state, ledger, catalog, mut turns) = fixture();
        state["runs"][0]["status"] = json!("completed");
        state["messages"].as_array_mut().unwrap().push(json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","contentDigest":super::sha256_hex(body),"createdAt":3000}));
        turns.get_mut("thread").unwrap()[0]["items"][2]["text"] = json!(output);
        hydrate(&mut state, &ledger, &catalog, &turns).unwrap();
        let restored = state["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["messageId"] == "reply")
            .unwrap();
        assert_eq!(restored["content"], body);
        assert_eq!(state["runs"][0]["output"], body);
    }
}

#[test]
fn restart_does_not_leave_an_unconfirmed_business_commit_pending_forever() {
    let (mut state, ledger, catalog, turns) = fixture();
    state["runs"][0]["status"] = json!("completed");
    hydrate(&mut state, &ledger, &catalog, &turns).unwrap();
    assert_eq!(state["runs"][0]["classificationStatus"], "failed");
    assert_eq!(state["runs"][0]["output"], "完成回复");
}

#[test]
fn restart_preserves_the_same_json_reply_as_live_completion() {
    for nonce in [Some("current"), None] {
        for raw in [
            r#"{"reply":"用户要求的字段","nonce":"example"}"#,
            r#"{"hello":"world"}"#,
            "```json\n{\"reply\":\"用户数据\"}\n```",
            "{ incomplete JSON requested by the user",
            r#"{"schemaVersion":1,"nonce":"current","reply":"内部正文"}"#,
            "正文\n{\"schemaVersion\":1,\"nonce\":\"current\",\"reply\":\"内部正文\"}",
        ] {
            for referenced in [false, true] {
                let body = nonce.map_or_else(
                    || raw.to_owned(),
                    |nonce| {
                        crate::work_classification::visible_reply(raw, nonce, false)
                            .unwrap()
                            .unwrap()
                    },
                );
                let body = body.trim();
                let (mut state, mut ledger, catalog, mut turns) = fixture();
                state["runs"][0]["status"] = json!("completed");
                state["runs"][0]["classificationNonce"] = json!(nonce);
                state["messages"].as_array_mut().unwrap().push(json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","contentDigest":super::sha256_hex(body),"createdAt":3000}));
                turns.get_mut("thread").unwrap()[0]["items"][2]["text"] = json!(raw);
                if referenced {
                    ledger["messageRefs"] = json!([{"messageId":"reply","nativeThreadId":"thread","nativeTurnId":"turn","nativeItemId":"a","bodyFormat":"native"}]);
                }
                hydrate(&mut state, &ledger, &catalog, &turns).unwrap();
                let restored = state["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|message| message["messageId"] == "reply")
                    .unwrap();
                assert_eq!(restored["content"], body);
                assert_eq!(state["runs"][0]["output"], body);
            }
        }
    }
}

#[test]
fn restart_preserves_verified_legacy_reply_without_changing_its_digest() {
    let (mut state, ledger, catalog, mut turns) = fixture();
    state["runs"][0]["status"] = json!("completed");
    state["runs"][0]["classificationNonce"] = json!("current");
    let hash = super::sha256_hex("旧版保存的正文");
    state["messages"].as_array_mut().unwrap().push(json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","contentDigest":hash,"createdAt":3000}));
    turns.get_mut("thread").unwrap()[0]["items"][2]["text"] =
        json!(r#"{"reply":"旧版保存的正文","nonce":"example"}"#);
    hydrate(&mut state, &ledger, &catalog, &turns).unwrap();
    let restored = state["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["messageId"] == "reply")
        .unwrap();
    assert_eq!(restored["content"], "旧版保存的正文");
    assert_eq!(restored["contentDigest"], hash);
    assert_eq!(state["runs"][0]["output"], "旧版保存的正文");
}

#[test]
fn reply_saved_with_a_copied_stale_envelope_restores_unchanged() {
    // 0.2.35 及更早版本把照抄的历史信封当作正文保存；之后的显示规则会隐藏它，恢复仍须还原原文。
    let (mut state, ledger, catalog, mut turns) = fixture();
    state["runs"][0]["status"] = json!("completed");
    state["runs"][0]["classificationNonce"] =
        json!("classification_6b1f3c1a-0f0e-4c8e-9a52-0d4a7c1e2b3f");
    let raw = r#"{"schemaVersion":1,"nonce":"classification_1dfe9531-1ca1-4e98-b11b-ffa70e0473ec","reply":"已生成热力图","decision":"create"}"#;
    let hash = super::sha256_hex(raw);
    state["messages"].as_array_mut().unwrap().push(json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","contentDigest":hash,"createdAt":3000}));
    turns.get_mut("thread").unwrap()[0]["items"][2]["text"] = json!(raw);
    hydrate(&mut state, &ledger, &catalog, &turns).unwrap();
    let restored = state["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["messageId"] == "reply")
        .unwrap();
    assert_eq!(restored["content"], raw);
    assert_eq!(restored["contentDigest"], hash);
}

#[test]
fn changed_native_json_is_not_accepted_by_either_reply_format() {
    let (mut state, ledger, catalog, mut turns) = fixture();
    state["runs"][0]["classificationNonce"] = json!("current");
    state["messages"].as_array_mut().unwrap().push(json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","contentDigest":super::sha256_hex("未改变的正文"),"createdAt":3000}));
    turns.get_mut("thread").unwrap()[0]["items"][2]["text"] =
        json!(r#"{"reply":"已经改变的正文","nonce":"example"}"#);
    assert_eq!(
        hydrate(&mut state, &ledger, &catalog, &turns).unwrap_err(),
        "native_history_content_mismatch:reply"
    );
}

#[test]
fn unclassified_completed_turn_restores_without_a_project_result_receipt() {
    let body = "普通对话正文\n{\"reply\":\"用户数据\",\"nonce\":\"历史示例\"}";
    for referenced in [false, true] {
        let (mut state, mut ledger, catalog, mut turns) = fixture();
        state["conversations"][0]["context"] = json!({"projectId":null});
        state["runs"][0]["status"] = json!("completed");
        state["runs"][0]["harnessProtocolVersion"] = json!(7);
        state["runs"][0]["classificationNonce"] = json!("unused-nonce");
        state["runs"][0]["classificationStatus"] = json!("applied");
        state["messages"].as_array_mut().unwrap().push(json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","contentDigest":super::sha256_hex(body),"createdAt":3000}));
        turns.get_mut("thread").unwrap()[0]["items"][2]["text"] = json!(format!("\n{body}\n"));
        if referenced {
            ledger["messageRefs"] = json!([{"messageId":"reply","nativeThreadId":"thread","nativeTurnId":"turn","nativeItemId":"a","bodyFormat":"native"}]);
        }
        hydrate(&mut state, &ledger, &catalog, &turns).unwrap();
        let restored = state["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["messageId"] == "reply")
            .unwrap();
        assert_eq!(restored["content"], body);
        assert_eq!(restored["contentDigest"], super::sha256_hex(body));
        assert_eq!(state["runs"][0]["output"], body);
        assert_eq!(state["runs"][0]["classificationStatus"], "applied");
        assert_eq!(state["runs"][0]["classificationNonce"], "unused-nonce");
        assert!(state["runs"][0].get("formalResultRequired").is_none());
        turns.get_mut("thread").unwrap()[0]["items"][2]["text"] = json!("已变化的正文");
        assert_eq!(
            hydrate(&mut state, &ledger, &catalog, &turns).unwrap_err(),
            "native_history_content_mismatch:reply"
        );
    }
}

#[test]
fn project_history_still_requires_its_formal_receipt_and_planning_preserves_plain_text() {
    for planning in [false, true] {
        let (mut state, ledger, catalog, turns) = fixture();
        state["conversations"][0]["context"] = json!({"projectId":"project"});
        state["runs"][0]["status"] = json!("completed");
        state["runs"][0]["planning"] = json!(planning);
        state["runs"][0]["harnessProtocolVersion"] = json!(7);
        state["runs"][0]["classificationNonce"] = json!("current");
        state["runs"][0]["classificationStatus"] = if planning {
            Value::Null
        } else {
            json!("applied")
        };
        state["messages"].as_array_mut().unwrap().push(json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","contentDigest":super::sha256_hex("完成回复"),"createdAt":3000}));
        let result = hydrate(&mut state, &ledger, &catalog, &turns);
        if planning {
            result.unwrap();
            assert_eq!(state["runs"][0]["output"], "完成回复");
            assert_eq!(state["runs"][0]["classificationStatus"], Value::Null);
        } else {
            assert_eq!(result.unwrap_err(), "workbench_result_history_missing");
        }
    }
}
