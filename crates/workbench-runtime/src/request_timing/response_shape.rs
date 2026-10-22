use serde_json::Value;

pub(super) fn observe(event: &Value) -> Vec<&'static str> {
    let mut stages = Vec::new();
    match event["type"].as_str() {
        Some("response.output_text.delta") if nonempty(&event["delta"]) => {
            stages.push("text_delta_nonempty");
        }
        Some("response.output_text.done") if nonempty(&event["text"]) => {
            stages.push("text_done_nonempty");
        }
        Some("response.output_item.added" | "response.output_item.done") => {
            if event["item"]["type"] == "message" {
                stages.push("message_item_observed");
                if message_has_text(&event["item"]) {
                    stages.push("message_item_text_nonempty");
                }
            }
        }
        Some("response.completed" | "response.incomplete" | "response.failed") => {
            let response = &event["response"];
            stages.push(match response["status"].as_str() {
                Some("completed") => "terminal_status_completed",
                Some("incomplete") => "terminal_status_incomplete",
                Some("failed") => "terminal_status_failed",
                Some("cancelled") => "terminal_status_cancelled",
                _ => "terminal_status_unknown",
            });
            if response.get("error").is_some_and(|value| !value.is_null()) {
                stages.push("terminal_error_present");
            }
            match response["incomplete_details"]["reason"].as_str() {
                Some("max_output_tokens") => stages.push("terminal_output_limit"),
                Some("content_filter") => stages.push("terminal_content_filter"),
                _ => {}
            }
            if let Some(output) = response["output"].as_array() {
                if output.is_empty() {
                    stages.push("terminal_output_empty");
                } else {
                    stages.push("terminal_output_nonempty");
                    if output.iter().any(message_has_text) {
                        stages.push("terminal_text_nonempty");
                    }
                    if output.iter().any(|item| item["type"] == "function_call") {
                        stages.push("terminal_function_call_present");
                    }
                }
            } else {
                stages.push("terminal_output_unknown");
            }
        }
        _ => {}
    }
    stages
}

fn nonempty(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.is_empty())
}

fn message_has_text(item: &Value) -> bool {
    item["type"] == "message"
        && item["content"].as_array().is_some_and(|content| {
            content
                .iter()
                .any(|part| part["type"] == "output_text" && nonempty(&part["text"]))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_completed_and_terminal_text_are_distinct_without_copying_content() {
        assert_eq!(
            observe(
                &json!({"type":"response.completed","response":{"status":"completed","output":[]}})
            ),
            ["terminal_status_completed", "terminal_output_empty"]
        );
        let secret = "synthetic-private-content";
        let event = json!({"type":"response.completed","response":{"status":"completed",
            "output":[{"type":"message","content":[{"type":"output_text","text":secret}]}],
            "headers":{"Authorization":secret},"model":secret,"id":secret}});
        let stages = observe(&event);
        assert_eq!(
            stages,
            [
                "terminal_status_completed",
                "terminal_output_nonempty",
                "terminal_text_nonempty"
            ]
        );
        assert!(!stages.join(",").contains(secret));
    }

    #[test]
    fn missing_output_unknown_status_and_safe_failures_remain_explicit() {
        assert_eq!(
            observe(
                &json!({"type":"response.completed","response":{"status":"private-value","output":null}})
            ),
            ["terminal_status_unknown", "terminal_output_unknown"]
        );
        assert_eq!(
            observe(
                &json!({"type":"response.incomplete","response":{"status":"incomplete","output":[],
            "incomplete_details":{"reason":"max_output_tokens"}}})
            ),
            [
                "terminal_status_incomplete",
                "terminal_output_limit",
                "terminal_output_empty"
            ]
        );
        assert_eq!(
            observe(
                &json!({"type":"response.failed","response":{"status":"failed","error":{"message":"private"}}})
            ),
            [
                "terminal_status_failed",
                "terminal_error_present",
                "terminal_output_unknown"
            ]
        );
    }

    #[test]
    fn text_events_require_actual_text_and_do_not_change_stream_data() {
        assert!(observe(&json!({"type":"response.output_text.delta","delta":""})).is_empty());
        assert_eq!(
            observe(&json!({"type":"response.output_text.delta","delta":"private"})),
            ["text_delta_nonempty"]
        );
        assert_eq!(
            observe(&json!({"type":"response.output_text.done","text":"private"})),
            ["text_done_nonempty"]
        );
        assert_eq!(
            observe(
                &json!({"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"private"}]}})
            ),
            ["message_item_observed", "message_item_text_nonempty"]
        );
        assert!(observe(&json!({"type":"private","delta":"private"})).is_empty());
    }
}
