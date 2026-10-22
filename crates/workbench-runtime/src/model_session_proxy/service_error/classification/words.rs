use serde_json::Value;

pub(super) const MAX_WORDS: usize = 32;
pub(super) const REASON_WORDS: &[&str] = &[
    "tool",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "function",
    "calling",
    "auto",
    "required",
    "none",
    "empty",
    "non-empty",
    "null",
    "list",
    "array",
    "should",
    "must",
    "not",
    "be",
    "at",
    "least",
    "one",
    "provided",
    "present",
    "requires",
    "without",
    "unsupported",
    "invalid",
    "expected",
    "only",
    "available",
    "when",
    "set",
    "specified",
    "can",
    "cannot",
    "if",
    "has",
    "have",
    "no",
    "found",
    "responses",
    "response",
    "request",
    "input",
    "message",
    "messages",
    "content",
    "prompt",
    "support",
    "supported",
    "model",
    "parameter",
    "parameters",
    "value",
    "format",
    "length",
    "minimum",
    "maximum",
    "limit",
    "tokens",
    "exceeds",
    "missing",
    "string",
    "boolean",
    "integer",
    "不能为空",
    "不支持",
    "未提供",
    "工具",
    "参数",
    "为空",
    "至少",
    "一个",
    "必须",
    "模型",
    "请求",
    "无效",
    "缺少",
    "格式",
    "类型",
];

fn allowed(value: &str) -> Option<&'static str> {
    super::parameter(value).or_else(|| REASON_WORDS.iter().copied().find(|word| *word == value))
}

pub(super) fn wire(values: &[&str]) -> String {
    let values = values
        .iter()
        .copied()
        .filter_map(allowed)
        .take(MAX_WORDS)
        .collect::<Vec<_>>();
    if values.is_empty() {
        "none".into()
    } else {
        format!("w:{}", values.join(","))
    }
}

pub(super) fn parse(value: &str) -> Option<Vec<&'static str>> {
    if value == "none" {
        return Some(Vec::new());
    }
    if value.len() > 1024 {
        return None;
    }
    let mut result = Vec::new();
    for token in value.strip_prefix("w:")?.split(',') {
        if result.len() >= MAX_WORDS {
            return None;
        }
        result.push(allowed(token)?);
    }
    (wire(&result) == value).then_some(result)
}

pub(super) fn collect(message: &str, values: &mut Vec<&'static str>) {
    collect_message(message, 0, values);
}
fn collect_message(message: &str, depth: usize, values: &mut Vec<&'static str>) {
    if values.len() >= MAX_WORDS || message.len() > super::MAX_MESSAGE {
        return;
    }
    let message = message.trim();
    if message.starts_with(['{', '[', '"'])
        && let Ok(value) = serde_json::from_str::<Value>(message)
    {
        if depth < 2 {
            collect_json(&value, depth + 1, 0, values);
        }
        return;
    }
    let normalized = message.to_ascii_lowercase();
    let mut offset = 0;
    while offset < normalized.len() && values.len() < MAX_WORDS {
        let remaining = &normalized[offset..];
        let first = remaining.chars().next().expect("offset is before the end");
        if token_character(first) {
            let length = remaining
                .find(|c: char| !token_character(c))
                .unwrap_or(remaining.len());
            if let Some(word) = allowed(remaining[..length].trim_matches('.')) {
                values.push(word);
            }
            offset += length;
        } else if let Some(word) = REASON_WORDS
            .iter()
            .copied()
            .find(|word| !word.is_ascii() && remaining.starts_with(word))
        {
            values.push(word);
            offset += word.len();
        } else {
            offset += first.len_utf8();
        }
    }
}
fn token_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
}
fn collect_json(value: &Value, depth: usize, fields: usize, words: &mut Vec<&'static str>) {
    if fields > 2 || words.len() >= MAX_WORDS {
        return;
    }
    match value {
        Value::String(message) => collect_message(message, depth, words),
        Value::Object(object) => {
            for key in ["error", "message", "detail", "msg"] {
                if let Some(value) = object.get(key) {
                    collect_json(value, depth, fields + 1, words);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter().take(super::MAX_DETAILS) {
                collect_json(item, depth, fields, words);
            }
        }
        _ => {}
    }
}
