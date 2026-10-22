//! 个人应用、历史会话和本地工具共用的中立快照与参数校验。
use contexthub_protocol::ApplicationSkillBinding;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationContextSelection {
    pub application_id: String,
    pub context_ids: Vec<String>,
    #[serde(default)]
    pub tool_names: Vec<String>,
    #[serde(default)]
    pub invocation_source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationModelTask {
    pub purpose: String,
    pub task_id: String,
    pub input_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_schema: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationAiContextSnapshot {
    pub application_id: String,
    pub application_name: String,
    #[serde(default)]
    pub application_ids: Vec<String>,
    #[serde(default)]
    pub restrict_to_applications: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_binding: Option<ApplicationSkillBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_task: Option<ApplicationModelTask>,
    #[serde(default)]
    pub browser_origins: Vec<String>,
    #[serde(default)]
    pub report_review: bool,
    #[serde(default)]
    pub local_material_paths: Vec<String>,
    pub version: String,
    pub page_revision: u64,
    pub contexts: Vec<Value>,
    pub actions: Vec<ApplicationUiActionSnapshot>,
    pub tool_names: Vec<String>,
    #[serde(default)]
    pub read_tools: Vec<ApplicationReadToolSnapshot>,
    #[serde(default)]
    pub write_tools: Vec<ApplicationWriteToolSnapshot>,
    #[serde(default)]
    pub searchable_applications: Vec<ApplicationSearchApplicationSnapshot>,
    pub tool_set_revision: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ApplicationDiscoveryNotice {
    pub status: String,
    pub selected_applications: usize,
    pub selected_tools: usize,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationReadToolSnapshot {
    pub application_id: String,
    pub tool_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationSearchApplicationSnapshot {
    pub application_id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub tools: Vec<ApplicationSearchToolSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationSearchToolSnapshot {
    pub name: String,
    pub title: String,
    pub description: String,
    pub operation_type: String,
    pub risk_level: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationWriteToolSnapshot {
    #[serde(default)]
    pub application_id: String,
    #[serde(default)]
    pub application_name: String,
    pub dynamic_name: String,
    pub tool_name: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub risk_level: String,
    pub timeout_ms: Option<u32>,
    #[serde(default)]
    pub confirmation_view: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUiActionSnapshot {
    pub action_id: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub risk_level: String,
    pub preconditions: Vec<String>,
    pub expected_effects: Vec<String>,
}

pub(crate) fn validate_tool_arguments(schema: &Value, value: &Value) -> Result<(), String> {
    validate_schema_value(schema, value, "$")
}

fn validate_schema_value(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(schemas) = schema.get("allOf").and_then(Value::as_array) {
        for child in schemas {
            validate_schema_value(child, value, path)?;
        }
    }
    if let Some(schemas) = schema.get("anyOf").and_then(Value::as_array)
        && !schemas
            .iter()
            .any(|child| validate_schema_value(child, value, path).is_ok())
    {
        return Err(format!("{path} 不符合 anyOf 中的任一结构"));
    }
    if let Some(schemas) = schema.get("oneOf").and_then(Value::as_array)
        && schemas
            .iter()
            .filter(|child| validate_schema_value(child, value, path).is_ok())
            .count()
            != 1
    {
        return Err(format!("{path} 必须且只能符合 oneOf 中的一种结构"));
    }
    if let Some(expected) = schema.get("const")
        && expected != value
    {
        return Err(format!("{path} 必须等于声明的固定值"));
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array)
        && !values.contains(value)
    {
        return Err(format!("{path} 不在允许的枚举值中"));
    }
    if let Some(declared_type) = schema.get("type")
        && !schema_type_matches(declared_type, value)
    {
        return Err(format!("{path} 的数据类型不符合 Schema"));
    }

    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64)
            && length < minimum
        {
            return Err(format!("{path} 字符长度不得少于 {minimum}"));
        }
        if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64)
            && length > maximum
        {
            return Err(format!("{path} 字符长度不得超过 {maximum}"));
        }
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            let expression =
                Regex::new(pattern).map_err(|_| format!("{path} 对应的 Schema 正则表达式无效"))?;
            if !expression.is_match(text) {
                return Err(schema_pattern_mismatch(schema, path));
            }
        }
        if let Some(format) = schema.get("format").and_then(Value::as_str)
            && !schema_format_matches(format, text)
        {
            return Err(format!("{path} 不符合 {format} 格式"));
        }
    }

    if let Some(number) = value.as_f64() {
        validate_schema_number(schema, number, path)?;
    }

    if let Some(items) = value.as_array() {
        let length = items.len() as u64;
        if let Some(minimum) = schema.get("minItems").and_then(Value::as_u64)
            && length < minimum
        {
            return Err(format!("{path} 项数不得少于 {minimum}"));
        }
        if let Some(maximum) = schema.get("maxItems").and_then(Value::as_u64)
            && length > maximum
        {
            return Err(format!("{path} 项数不得超过 {maximum}"));
        }
        if schema.get("uniqueItems").and_then(Value::as_bool) == Some(true)
            && items
                .iter()
                .enumerate()
                .any(|(index, item)| items[..index].contains(item))
        {
            return Err(format!("{path} 不得包含重复项"));
        }
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in items.iter().enumerate() {
                validate_schema_value(item_schema, item, &format!("{path}[{index}]"))?;
            }
        }
    }

    if let Some(object) = value.as_object() {
        validate_schema_object(schema, object, path)?;
    }
    Ok(())
}

fn schema_pattern_mismatch(schema: &Value, path: &str) -> String {
    let detail = schema
        .get("description")
        .and_then(Value::as_str)
        .map(|description| description.chars().take(500).collect::<String>())
        .filter(|description| !description.is_empty());
    match detail {
        Some(detail) => format!("{path} 格式不符合 Schema：{detail}"),
        None => format!("{path} 格式不符合 Schema"),
    }
}

fn validate_schema_number(schema: &Value, number: f64, path: &str) -> Result<(), String> {
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
        && number < minimum
    {
        return Err(format!("{path} 不得小于 {minimum}"));
    }
    if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64)
        && number > maximum
    {
        return Err(format!("{path} 不得大于 {maximum}"));
    }
    Ok(())
}

fn validate_schema_object(
    schema: &Value,
    object: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<(), String> {
    let length = object.len() as u64;
    if let Some(minimum) = schema.get("minProperties").and_then(Value::as_u64)
        && length < minimum
    {
        return Err(format!("{path} 字段数不得少于 {minimum}"));
    }
    if let Some(maximum) = schema.get("maxProperties").and_then(Value::as_u64)
        && length > maximum
    {
        return Err(format!("{path} 字段数不得超过 {maximum}"));
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                return Err(format!("{path}.{key} 为必填字段"));
            }
        }
    }
    let properties = schema.get("properties").and_then(Value::as_object);
    for (key, child_value) in object {
        let child_path = format!("{path}.{key}");
        if let Some(name_schema) = schema.get("propertyNames") {
            validate_schema_value(name_schema, &Value::String(key.clone()), &child_path)?;
        }
        if let Some(child_schema) = properties.and_then(|items| items.get(key)) {
            validate_schema_value(child_schema, child_value, &child_path)?;
            continue;
        }
        match schema.get("additionalProperties") {
            Some(Value::Bool(false)) => {
                return Err(format!("{child_path} 未在 Schema 中声明"));
            }
            Some(additional_schema) if additional_schema.is_object() => {
                validate_schema_value(additional_schema, child_value, &child_path)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn schema_type_matches(declared_type: &Value, value: &Value) -> bool {
    let matches = |name: &str| match name {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => false,
    };
    declared_type.as_str().is_some_and(matches)
        || declared_type
            .as_array()
            .is_some_and(|types| types.iter().filter_map(Value::as_str).any(matches))
}

fn schema_format_matches(format: &str, value: &str) -> bool {
    match format {
        "uuid" => uuid::Uuid::parse_str(value).is_ok(),
        "date" => chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok(),
        "date-time" => chrono::DateTime::parse_from_rfc3339(value).is_ok(),
        "uri" => reqwest::Url::parse(value).is_ok(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tool_argument_validation_checks_declared_formats() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "recordId": { "type": "string", "format": "uuid" }
            },
            "required": ["recordId"]
        });
        let error = super::validate_tool_arguments(
            &schema,
            &serde_json::json!({ "recordId": "placeholder" }),
        )
        .unwrap_err();
        assert!(error.contains("$.recordId"));
        assert!(error.contains("uuid"));
    }

    #[test]
    fn personal_schema_constraints_remain_enforced() {
        let schema = serde_json::json!({"type":"object","required":["name","items"],"additionalProperties":false,"properties":{"name":{"type":"string","minLength":2,"maxLength":4,"pattern":"^[a-z]+$"},"items":{"type":"array","minItems":1,"maxItems":2,"uniqueItems":true,"items":{"type":"integer","minimum":1,"maximum":5}}}});
        assert!(
            super::validate_tool_arguments(
                &schema,
                &serde_json::json!({"name":"valid","items":[1]})
            )
            .is_err()
        );
        assert!(
            super::validate_tool_arguments(
                &schema,
                &serde_json::json!({"name":"ok","items":[1,3]})
            )
            .is_ok()
        );
        for value in [
            serde_json::json!({"name":"A","items":[1]}),
            serde_json::json!({"name":"ok","items":[1,1]}),
            serde_json::json!({"name":"ok","items":[6]}),
            serde_json::json!({"name":"ok","items":[]}),
            serde_json::json!({"name":"ok","items":[1],"extra":true}),
        ] {
            assert!(super::validate_tool_arguments(&schema, &value).is_err());
        }
    }
}
