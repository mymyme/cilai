use super::*;

fn normalized(value: &Value) -> Value {
    serde_json::from_slice(
        &constrained_request(&serde_json::to_vec(value).unwrap(), "m", None).unwrap(),
    )
    .unwrap()
}

fn request() -> Value {
    json!({"model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"synthetic fixture"}]}],
        "stream":true,"store":false,"tools":[],"tool_choice":"auto","parallel_tool_calls":true,
        "reasoning":{},"include":["reasoning.encrypted_content"],"temperature":0.2,
        "prompt_cache_key":"synthetic-key","client_metadata":{"fixture":true}})
}

#[test]
fn empty_auto_tools_remove_only_three_inapplicable_fields_and_preserve_every_other_value() {
    for choice in [None, Some(json!("auto"))] {
        for parallel in [None, Some(json!(true)), Some(json!(false))] {
            let mut original = request();
            for (field, value) in [
                ("tool_choice", choice.clone()),
                ("parallel_tool_calls", parallel),
            ] {
                let object = original.as_object_mut().unwrap();
                if let Some(value) = value {
                    object.insert(field.into(), value);
                } else {
                    object.remove(field);
                }
            }
            let mut expected = original.clone();
            for field in ["tools", "tool_choice", "parallel_tool_calls"] {
                expected.as_object_mut().unwrap().remove(field);
            }
            assert_eq!(normalized(&original), expected);
        }
    }
}

#[test]
fn declared_nonempty_tools_and_all_explicit_choices_are_preserved_in_full() {
    for choice in [
        None,
        Some(json!("auto")),
        Some(json!("none")),
        Some(json!("required")),
        Some(json!({"type":"function","name":"safe_tool"})),
        Some(
            json!({"type":"allowed_tools","mode":"auto","tools":[{"type":"function","name":"safe_tool"}]}),
        ),
    ] {
        let mut original = request();
        original["tools"] = json!([{"type":"function","name":"safe_tool","description":"synthetic",
            "strict":true,"parameters":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}}]);
        if let Some(choice) = choice {
            original["tool_choice"] = choice;
        } else {
            original.as_object_mut().unwrap().remove("tool_choice");
        }
        assert_eq!(normalized(&original), original);
    }
}

#[test]
fn empty_tools_with_explicit_or_malformed_choice_are_not_silently_changed() {
    for choice in [
        json!("none"),
        json!("required"),
        json!({"type":"function","name":"safe_tool"}),
        Value::Null,
        json!(true),
        json!(1),
        json!([]),
        json!("future_choice"),
    ] {
        let mut original = request();
        original["tool_choice"] = choice;
        assert_eq!(normalized(&original), original);
    }
}

#[test]
fn missing_null_or_invalid_tool_fields_are_not_treated_as_empty_arrays() {
    for tools in [
        None,
        Some(Value::Null),
        Some(json!({})),
        Some(json!("invalid")),
        Some(json!(false)),
    ] {
        let mut original = request();
        if let Some(tools) = tools {
            original["tools"] = tools;
        } else {
            original.as_object_mut().unwrap().remove("tools");
        }
        assert_eq!(normalized(&original), original);
    }
    for parallel in [Value::Null, json!("true"), json!(1), json!({})] {
        let mut original = request();
        original["parallel_tool_calls"] = parallel;
        assert_eq!(normalized(&original), original);
    }
}

#[test]
fn normalization_keeps_model_pinning_fallback_and_original_page_tool_gates() {
    let original = request();
    assert_eq!(
        constrained_request(&serde_json::to_vec(&original).unwrap(), "other", None).unwrap_err(),
        "MODEL_SESSION_PROXY_MODEL_SCOPE_INVALID"
    );
    let mut wrong_route = original.clone();
    wrong_route["route"] = json!("fallback");
    assert!(constrained_request(&serde_json::to_vec(&wrong_route).unwrap(), "m", None).is_err());
    let endpoint = Url::parse("https://openrouter.ai/api/v1").unwrap();
    let result: Value = serde_json::from_slice(
        &constrained_request(
            &serde_json::to_vec(&original).unwrap(),
            "m",
            Some(&endpoint),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result["provider"]["allow_fallbacks"], false);
    assert!(result.get("tools").is_none());
    assert!(validate_page_tool_request(&serde_json::to_vec(&original).unwrap(), &[]).is_ok());
    assert_eq!(
        validate_page_tool_request(
            &serde_json::to_vec(&original).unwrap(),
            &["authorized_tool".into()]
        ),
        Err("PERSONAL_PAGE_NATIVE_TOOLS_NOT_ISOLATED")
    );
    let mut injected = original;
    injected["tools"] = json!([{"type":"function","name":"unauthorized_tool"}]);
    assert!(validate_page_tool_request(&serde_json::to_vec(&injected).unwrap(), &[]).is_err());
    assert_eq!(normalized(&injected), injected);
}
