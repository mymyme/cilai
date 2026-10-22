use serde::Serialize;
use serde_json::Value;

mod words;

const MAX_HINTS: usize = 8;
const MAX_DETAILS: usize = 16;
const MAX_MESSAGE: usize = 2048;
const PARAMETERS: &[&str] = &[
    "model",
    "input",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "stream",
    "store",
    "include",
    "reasoning.effort",
    "reasoning.summary",
    "reasoning",
    "text.format",
    "text.verbosity",
    "text",
    "max_output_tokens",
    "previous_response_id",
    "truncation",
    "temperature",
    "top_p",
    "service_tier",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParameterHint {
    pub parameter: &'static str,
    pub source: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Classification {
    pub reason: &'static str,
    pub parameter_hints: Vec<ParameterHint>,
    pub shape: &'static str,
    pub reason_words: Vec<&'static str>,
}

pub(crate) fn unknown() -> Classification {
    Classification {
        reason: "unknown",
        parameter_hints: Vec::new(),
        shape: "other",
        reason_words: Vec::new(),
    }
}

impl Classification {
    pub(crate) fn wire_words(&self) -> String {
        words::wire(&self.reason_words)
    }
    pub(crate) fn wire_hints(&self) -> String {
        let mut hints = self
            .parameter_hints
            .iter()
            .filter(|hint| parameter(hint.parameter).is_some() && source(hint.source).is_some())
            .map(|hint| (hint.parameter, hint.source))
            .collect::<Vec<_>>();
        hints.sort_unstable();
        hints.dedup();
        if hints.is_empty() {
            return "none".into();
        }
        hints
            .into_iter()
            .take(MAX_HINTS)
            .map(|(parameter, source)| format!("{parameter}:{source}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

pub(crate) fn from_wire(
    reason: &str,
    shape: &str,
    hints: &str,
    words: &str,
) -> Option<Classification> {
    let mut result = Classification {
        reason: allowed_reason(reason)?,
        shape: allowed_shape(shape)?,
        parameter_hints: Vec::new(),
        reason_words: words::parse(words)?,
    };
    if hints != "none" {
        if hints.len() > 512 {
            return None;
        }
        for hint in hints.split(',') {
            let (name, evidence) = hint.split_once(':')?;
            if result.parameter_hints.len() >= MAX_HINTS {
                return None;
            }
            result.parameter_hints.push(ParameterHint {
                parameter: parameter(name)?,
                source: source(evidence)?,
            });
        }
    }
    (result.wire_hints() == hints).then_some(result)
}

fn parameter(value: &str) -> Option<&'static str> {
    PARAMETERS.iter().copied().find(|item| *item == value)
}
fn source(value: &str) -> Option<&'static str> {
    match value {
        "explicit_param" => Some("explicit_param"),
        "validation_location" => Some("validation_location"),
        "message_keyword" => Some("message_keyword"),
        _ => None,
    }
}
fn allowed_shape(value: &str) -> Option<&'static str> {
    match value {
        "error_object" => Some("error_object"),
        "validation_detail" => Some("validation_detail"),
        "error_string" => Some("error_string"),
        "mixed" => Some("mixed"),
        "other" => Some("other"),
        _ => None,
    }
}
fn allowed_reason(value: &str) -> Option<&'static str> {
    match value {
        "unknown" => Some("unknown"),
        "unsupported_parameter" => Some("unsupported_parameter"),
        "unknown_parameter" => Some("unknown_parameter"),
        "missing_required_parameter" => Some("missing_required_parameter"),
        "invalid_type" => Some("invalid_type"),
        "invalid_value" => Some("invalid_value"),
        "incompatible_parameters" => Some("incompatible_parameters"),
        "tools_required" => Some("tools_required"),
        "empty_tools_rejected" => Some("empty_tools_rejected"),
        "context_length_exceeded" => Some("context_length_exceeded"),
        "authentication_rejected" => Some("authentication_rejected"),
        _ => None,
    }
}
fn reason_from_code(value: &str) -> Option<&'static str> {
    match value {
        "unsupported_parameter" => Some("unsupported_parameter"),
        "unknown_parameter" | "extra_forbidden" => Some("unknown_parameter"),
        "missing_required_parameter" | "missing_required_argument" | "missing" => {
            Some("missing_required_parameter")
        }
        "invalid_type" | "string_type" | "list_type" | "dict_type" | "bool_type" | "int_type"
        | "float_type" => Some("invalid_type"),
        "invalid_value" | "unsupported_value" | "literal_error" | "bool_parsing"
        | "int_parsing" | "float_parsing" | "too_short" | "too_long" => Some("invalid_value"),
        "incompatible_parameters" | "mutually_exclusive_parameters" => {
            Some("incompatible_parameters")
        }
        "tools_required" | "tool_choice_requires_tools" => Some("tools_required"),
        "empty_tools_not_allowed" => Some("empty_tools_rejected"),
        "context_length_exceeded" => Some("context_length_exceeded"),
        "invalid_api_key" | "authentication_error" => Some("authentication_rejected"),
        _ => None,
    }
}

struct Collector {
    classification: Classification,
    reason: Option<&'static str>,
    conflicted: bool,
}
impl Collector {
    fn reason(&mut self, reason: Option<&'static str>) {
        if let Some(reason) = reason {
            if self.reason.is_some_and(|prior| prior != reason) {
                self.conflicted = true;
            }
            self.reason = Some(reason);
        }
    }
    fn hint(&mut self, name: Option<&'static str>, source: &'static str) {
        if let Some(parameter) = name {
            let hint = ParameterHint { parameter, source };
            if self.classification.parameter_hints.len() < MAX_HINTS
                && !self.classification.parameter_hints.contains(&hint)
            {
                self.classification.parameter_hints.push(hint);
            }
        }
    }
    fn message(&mut self, value: Option<&str>) {
        let Some(value) = value.filter(|value| value.len() <= MAX_MESSAGE) else {
            return;
        };
        words::collect(value, &mut self.classification.reason_words);
        let normalized = value.trim().to_ascii_lowercase();
        let clue = message_clue(&normalized);
        self.reason(clue.0);
        for name in clue.1 {
            self.hint(Some(name), "message_keyword");
        }
    }
}

pub(crate) fn classify(value: &Value) -> Classification {
    let error = &value["error"];
    let details = value["detail"].as_array();
    let shape = match (error.is_object() || error.is_string(), details.is_some()) {
        (true, true) => "mixed",
        (false, true) => "validation_detail",
        (true, false) if error.is_object() => "error_object",
        _ if error.is_string() || value["detail"].is_string() => "error_string",
        _ => "other",
    };
    let mut collector = Collector {
        classification: Classification { shape, ..unknown() },
        reason: None,
        conflicted: false,
    };
    collector.hint(
        error["param"].as_str().and_then(parameter),
        "explicit_param",
    );
    collector.reason(error["code"].as_str().and_then(reason_from_code));
    collector.reason(error["type"].as_str().and_then(reason_from_code));
    if let Some(details) = details {
        for detail in details.iter().take(MAX_DETAILS) {
            collector.hint(location(&detail["loc"]), "validation_location");
            collector.reason(detail["type"].as_str().and_then(reason_from_code));
        }
    }
    collector.message(error["message"].as_str());
    collector.message(error.as_str());
    collector.message(value["detail"].as_str());
    if let Some(details) = details {
        for detail in details.iter().take(MAX_DETAILS) {
            collector.message(detail["msg"].as_str());
        }
    }
    collector.classification.reason = if collector.conflicted {
        "unknown"
    } else {
        collector.reason.unwrap_or("unknown")
    };
    collector
        .classification
        .parameter_hints
        .sort_unstable_by_key(|hint| (hint.parameter, hint.source));
    collector.classification
}

fn location(value: &Value) -> Option<&'static str> {
    let mut parts = value.as_array()?.as_slice();
    if parts.first().and_then(Value::as_str) == Some("body") {
        parts = &parts[1..];
    }
    if parts.is_empty() || parts.len() > 8 {
        return None;
    }
    let base = parameter(parts[0].as_str()?)?;
    let mut names = vec![base];
    for part in &parts[1..] {
        if part.as_u64().is_some() {
            continue;
        }
        let name = part.as_str()?;
        if !matches!(
            name,
            "type"
                | "name"
                | "function"
                | "description"
                | "parameters"
                | "strict"
                | "role"
                | "content"
                | "text"
                | "effort"
                | "summary"
                | "format"
                | "verbosity"
        ) {
            return None;
        }
        names.push(name);
    }
    if names.len() == 2
        && let Some(exact) = parameter(&names.join("."))
    {
        return Some(exact);
    }
    Some(base)
}

fn parameter_after(value: &str) -> Option<&'static str> {
    let value = value.trim_start();
    for name in PARAMETERS {
        for quote in ["'", "\"", "`", ""] {
            let prefix = format!("{quote}{name}{quote}");
            if let Some(tail) = value.strip_prefix(&prefix)
                && (tail.is_empty()
                    || tail == "."
                    || tail.starts_with(". ")
                    || tail.starts_with(|character: char| {
                        character.is_ascii_whitespace() || matches!(character, ':' | ',' | ';')
                    }))
            {
                return Some(name);
            }
        }
    }
    None
}
fn message_clue(value: &str) -> (Option<&'static str>, Vec<&'static str>) {
    for (prefix, reason) in [
        ("unsupported parameter:", "unsupported_parameter"),
        ("unsupported parameter ", "unsupported_parameter"),
        ("unknown parameter:", "unknown_parameter"),
        ("unknown parameter ", "unknown_parameter"),
        (
            "unrecognized request argument supplied:",
            "unknown_parameter",
        ),
        ("missing required parameter:", "missing_required_parameter"),
        ("missing required argument:", "missing_required_parameter"),
        ("invalid type for parameter ", "invalid_type"),
        ("invalid value for parameter ", "invalid_value"),
    ] {
        if let Some(tail) = value.strip_prefix(prefix)
            && let Some(name) = parameter_after(tail)
        {
            return (Some(reason), vec![name]);
        }
    }
    let plain = value.replace(['\'', '"', '`'], "");
    let plain = plain
        .strip_prefix("value error, ")
        .unwrap_or(&plain)
        .trim_end_matches('.');
    match plain {
        "tool_choice requires tools"
        | "tool_choice requires at least one tool"
        | "tool_choice cannot be used without tools"
        | "tool_choice is only allowed when tools are present"
        | "tool_choice is only allowed when tools are provided"
        | "tool_choice is only supported when tools are provided"
        | "tool_choice cannot be set when tools are not provided"
        | "tool_choice is not supported without tools" => {
            (Some("tools_required"), vec!["tool_choice", "tools"])
        }
        "tool_choice must be none when tools is empty"
        | "when tools is empty, tool_choice must be none" => (
            Some("incompatible_parameters"),
            vec!["tool_choice", "tools"],
        ),
        "parallel_tool_calls requires tools"
        | "parallel_tool_calls is not supported without tools"
        | "parallel_tool_calls cannot be true when tools is empty" => (
            Some("incompatible_parameters"),
            vec!["parallel_tool_calls", "tools"],
        ),
        "tools must not be empty"
        | "tools must contain at least one tool"
        | "tools must have at least one item"
        | "tools must have at least 1 item"
        | "tools must be non-empty"
        | "tools must be non empty"
        | "tools must contain at least one item (min_items=1)" => {
            (Some("empty_tools_rejected"), vec!["tools"])
        }
        _ if plain.starts_with("invalid value:") || plain.starts_with("unsupported value:") => {
            (Some("invalid_value"), Vec::new())
        }
        _ => (None, Vec::new()),
    }
}

#[cfg(test)]
mod tests;
