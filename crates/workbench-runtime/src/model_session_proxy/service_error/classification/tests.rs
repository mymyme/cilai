use super::{Classification, ParameterHint, classify, from_wire, unknown};
use serde_json::json;

#[test]
fn structured_code_type_and_parameter_are_fixed_and_source_is_explicit() {
    let result = classify(
        &json!({"error":{"type":"unsupported_parameter","param":"tool_choice","message":"private-provider-message"}}),
    );
    assert_eq!(result.reason, "unsupported_parameter");
    assert_eq!(result.shape, "error_object");
    assert_eq!(
        result.parameter_hints,
        vec![ParameterHint {
            parameter: "tool_choice",
            source: "explicit_param"
        }]
    );
    assert_eq!(
        from_wire(
            result.reason,
            result.shape,
            &result.wire_hints(),
            &result.wire_words()
        ),
        Some(result)
    );
}

#[test]
fn pydantic_locations_and_types_do_not_expose_dynamic_path_segments() {
    let result = classify(
        &json!({"detail":[{"loc":["body","tools",0,"type"],"type":"missing","msg":"private-provider-message","input":"private-input"}]}),
    );
    assert_eq!(result.reason, "missing_required_parameter");
    assert_eq!(result.shape, "validation_detail");
    assert_eq!(
        result.parameter_hints,
        vec![ParameterHint {
            parameter: "tools",
            source: "validation_location"
        }]
    );
    let nested =
        classify(&json!({"detail":[{"loc":["body","reasoning","effort"],"type":"literal_error"}]}));
    assert_eq!(nested.reason, "invalid_value");
    assert_eq!(nested.wire_hints(), "reasoning.effort:validation_location");
    for loc in [
        json!(["body", "tools", 0, "private-field"]),
        json!(["query", "tools"]),
        json!(["body", "tools", -1]),
        json!(["body", "tools", "0"]),
    ] {
        assert!(
            classify(&json!({"detail":[{"loc":loc,"type":"missing"}]}))
                .parameter_hints
                .is_empty()
        );
    }
}

#[test]
fn message_only_parameter_clues_do_not_become_explicit_parameters() {
    for (message, reason, hints) in [
        (
            "Unsupported parameter: 'tool_choice'.",
            "unsupported_parameter",
            "tool_choice:message_keyword",
        ),
        (
            "Unknown parameter: parallel_tool_calls.",
            "unknown_parameter",
            "parallel_tool_calls:message_keyword",
        ),
        (
            "Missing required parameter: `tools`.",
            "missing_required_parameter",
            "tools:message_keyword",
        ),
        (
            "Invalid type for parameter 'stream': expected a boolean.",
            "invalid_type",
            "stream:message_keyword",
        ),
    ] {
        let result = classify(&json!({"error":{"message":message}}));
        assert_eq!(result.reason, reason);
        assert_eq!(result.wire_hints(), hints);
    }
    for message in [
        "tool_choice was included in private-provider-message",
        "Unsupported parameter: tool_choice_PRIVATE",
        "Unknown parameter: 'private tools'",
        "Unsupported parameter: tool_choice.private",
        "The URL https://private.invalid/tools has a secret tool_choice",
    ] {
        let result = classify(&json!({"error":{"message":message}}));
        assert_eq!(result.reason, "unknown");
        assert!(result.parameter_hints.is_empty());
    }
}

#[test]
fn empty_tools_and_parallel_requirements_have_fixed_conditional_clues() {
    for (message, reason, hints) in [
        (
            "'tool_choice' is only allowed when 'tools' are provided.",
            "tools_required",
            "tool_choice:message_keyword,tools:message_keyword",
        ),
        (
            "tool_choice cannot be used without tools",
            "tools_required",
            "tool_choice:message_keyword,tools:message_keyword",
        ),
        (
            "tool_choice must be 'none' when tools is empty",
            "incompatible_parameters",
            "tool_choice:message_keyword,tools:message_keyword",
        ),
        (
            "tools must have at least one item",
            "empty_tools_rejected",
            "tools:message_keyword",
        ),
        (
            "tools must be non-empty",
            "empty_tools_rejected",
            "tools:message_keyword",
        ),
        (
            "parallel_tool_calls requires tools",
            "incompatible_parameters",
            "parallel_tool_calls:message_keyword,tools:message_keyword",
        ),
    ] {
        let result = classify(&json!({"error":{"message":message}}));
        assert_eq!(result.reason, reason, "fixed template not recognized");
        assert_eq!(result.wire_hints(), hints);
    }
}

#[test]
fn contradictory_fixed_evidence_keeps_reason_unknown_without_losing_sources() {
    let result = classify(
        &json!({"error":{"code":"unsupported_parameter","param":"tools","message":"Missing required parameter: tools."},"detail":[{"loc":["body","tools"],"type":"missing"}]}),
    );
    assert_eq!(result.reason, "unknown");
    assert_eq!(result.shape, "mixed");
    assert_eq!(
        result.wire_hints(),
        "tools:explicit_param,tools:message_keyword,tools:validation_location"
    );
}

#[test]
fn hints_are_bounded_sorted_and_deduplicated() {
    let details = (0..32)
        .map(|_| json!({"loc":["body","tools",0,"type"],"type":"missing"}))
        .collect::<Vec<_>>();
    let result = classify(&json!({"detail":details}));
    assert_eq!(result.parameter_hints.len(), 1);
    assert_eq!(result.wire_hints(), "tools:validation_location");
    let details = super::PARAMETERS
        .iter()
        .map(|parameter| json!({"loc":["body",parameter],"type":"missing"}))
        .collect::<Vec<_>>();
    let result = classify(&json!({"detail":details}));
    assert_eq!(result.parameter_hints.len(), 8);
    let mut names = result
        .parameter_hints
        .iter()
        .map(|hint| hint.parameter)
        .collect::<Vec<_>>();
    let original = names.clone();
    names.sort_unstable();
    assert_eq!(names, original);
}

#[test]
fn free_provider_text_headers_and_identifiers_never_survive_classification() {
    let result = classify(
        &json!({"error":{"type":"private-type","code":"private-code","param":"private-param","message":"private-message Bearer secret-value","headers":{"Authorization":"private-header"}},"request_id":"private-id","url":"https://private.invalid","detail":[{"loc":["body","private-field"],"type":"private-validation","msg":"private-msg","input":"private-input"}]}),
    );
    let public = serde_json::to_string(&result).unwrap();
    for private in [
        "private",
        "Bearer",
        "Authorization",
        "secret-value",
        "request_id",
        "https://",
    ] {
        assert!(!public.contains(private));
    }
    assert_eq!(result.reason, "unknown");
    assert_eq!(result.wire_hints(), "none");
}

#[test]
fn unrecognized_and_oversized_messages_remain_unknown() {
    for value in [
        json!(null),
        json!("Unsupported parameter: tools"),
        json!({"error":{"message":"x".repeat(super::MAX_MESSAGE+1)}}),
    ] {
        let result = classify(&value);
        assert_eq!(result.reason, "unknown");
        assert!(result.parameter_hints.is_empty());
    }
    let value = classify(&json!({"detail":"tools must not be empty"}));
    assert_eq!(value.shape, "error_string");
    assert_eq!(value.reason, "empty_tools_rejected");
}

#[test]
fn canonical_wire_parser_rejects_unknown_free_duplicate_and_unsorted_fields() {
    assert_eq!(
        from_wire("unknown", "other", "none", "none"),
        Some(unknown())
    );
    for (reason, shape, hints) in [
        ("private", "other", "none"),
        ("unknown", "private", "none"),
        ("unknown", "other", ""),
        ("unknown", "other", "tools:private"),
        ("unknown", "other", "private:explicit_param"),
        (
            "unknown",
            "other",
            "tools:explicit_param,tools:explicit_param",
        ),
        (
            "unknown",
            "other",
            "tools:explicit_param,stream:explicit_param",
        ),
        ("unknown", "other", "tools:explicit_param trailing"),
        ("unknown", "other", "tools:explicit_param:private"),
    ] {
        assert_eq!(from_wire(reason, shape, hints, "none"), None);
    }
    let oversized = "tools:explicit_param,".repeat(30);
    assert_eq!(from_wire("unknown", "other", &oversized, "none"), None);
}

#[test]
fn wire_projection_drops_invalid_compiled_hints_and_never_exceeds_eight() {
    let value = Classification {
        reason: "unknown",
        shape: "other",
        reason_words: Vec::new(),
        parameter_hints: vec![
            ParameterHint {
                parameter: "private",
                source: "explicit_param",
            },
            ParameterHint {
                parameter: "tools",
                source: "private",
            },
        ],
    };
    assert_eq!(value.wire_hints(), "none");
    for reason in [
        "unsupported_parameter",
        "unknown_parameter",
        "missing_required_parameter",
        "invalid_type",
        "invalid_value",
        "incompatible_parameters",
        "tools_required",
        "empty_tools_rejected",
        "context_length_exceeded",
        "authentication_rejected",
    ] {
        assert!(from_wire(reason, "error_object", "none", "none").is_some());
    }
}

#[test]
fn fixed_words_keep_order_repetition_and_literal_none_has_unambiguous_wire() {
    let result = classify(
        &json!({"error":{"message":"tool_choice must be none when tools is empty; tools must be non-empty"}}),
    );
    assert_eq!(
        result.reason_words,
        vec![
            "tool_choice",
            "must",
            "be",
            "none",
            "when",
            "tools",
            "empty",
            "tools",
            "must",
            "be",
            "non-empty"
        ]
    );
    let one = from_wire("unknown", "other", "none", "w:none").unwrap();
    assert_eq!(one.reason_words, vec!["none"]);
    assert_eq!(one.wire_words(), "w:none");
    assert_eq!(
        from_wire("unknown", "other", "none", "none"),
        Some(unknown())
    );
    assert_eq!(
        from_wire(
            result.reason,
            result.shape,
            &result.wire_hints(),
            &result.wire_words()
        ),
        Some(result)
    );
}

#[test]
fn quoted_non_json_message_keeps_only_fixed_semantic_words() {
    let result =
        classify(&json!({"error":{"message":"\"tool_choice\" must be none when tools is empty"}}));
    assert_eq!(
        result.reason_words,
        vec![
            "tool_choice",
            "must",
            "be",
            "none",
            "when",
            "tools",
            "empty"
        ]
    );
    let result = classify(
        &json!({"error":{"message":"tool_choice_PRIVATE tool_choice123 private-word 2048 abc123 https://private.invalid/secret"}}),
    );
    assert!(result.reason_words.is_empty());
}

#[test]
fn nested_message_json_reads_only_known_semantic_fields_to_bounded_depth() {
    let nested=json!({"error":{"message":"tools must not be empty"},"headers":{"message":"model required"},"request_id":"prompt supported"}).to_string();
    let result = classify(&json!({"error":{"message":nested}}));
    assert_eq!(
        result.reason_words,
        vec!["tools", "must", "not", "be", "empty"]
    );
    let pydantic =
        json!({"detail":[{"msg":"tools requires at least one function","input":"model prompt"}]})
            .to_string();
    assert_eq!(
        classify(&json!({"error":{"message":pydantic}})).reason_words,
        vec!["tools", "requires", "at", "least", "one", "function"]
    );
    let deep = json!({"error":{"error":{"message":"tools required"}}}).to_string();
    assert!(
        classify(&json!({"error":{"message":deep}}))
            .reason_words
            .is_empty()
    );
    let once = serde_json::to_string("tools required").unwrap();
    let twice = serde_json::to_string(&once).unwrap();
    let third = serde_json::to_string(&twice).unwrap();
    assert_eq!(
        classify(&json!({"error":{"message":twice}})).reason_words,
        vec!["tools", "required"]
    );
    assert!(
        classify(&json!({"error":{"message":third}}))
            .reason_words
            .is_empty()
    );
}

#[test]
fn fixed_chinese_phrases_do_not_output_arbitrary_han_characters() {
    let result = classify(
        &json!({"error":{"message":"工具参数不能为空；至少一个模型；任意私有内容不输出"}}),
    );
    assert_eq!(
        result.reason_words,
        vec!["工具", "参数", "不能为空", "至少", "一个", "模型"]
    );
    assert!(!result.wire_words().contains("私有"));
}

#[test]
fn word_projection_and_wire_are_capped_and_reject_unknown_tokens() {
    let result = classify(&json!({"error":{"message":"tools ".repeat(50)}}));
    assert_eq!(result.reason_words, vec!["tools"; 32]);
    assert_eq!(
        from_wire("unknown", "error_object", "none", &result.wire_words())
            .unwrap()
            .reason_words
            .len(),
        32
    );
    for words in [
        "w:",
        "w:private",
        "w:42",
        "w:tools, private",
        "w:tools,",
        "none,tools",
        "tools",
        "w:tools\nrequired",
    ] {
        assert!(from_wire("unknown", "other", "none", words).is_none());
    }
    let too_many = format!("w:{}", vec!["tools"; 33].join(","));
    assert!(from_wire("unknown", "other", "none", &too_many).is_none());
    assert_eq!(
        from_wire("unknown", "other", "none", "w:tools,tools")
            .unwrap()
            .reason_words,
        vec!["tools", "tools"]
    );
}
