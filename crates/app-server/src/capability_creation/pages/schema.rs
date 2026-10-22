use super::super::ServiceError;
use serde_json::Value;
use std::collections::HashSet;

fn invalid() -> ServiceError {
    ServiceError::new("personal_page_schema_invalid")
}

pub(super) fn validate(schema: &Value) -> Result<(), ServiceError> {
    if schema.get("type").and_then(Value::as_str) != Some("object")
        || schema.to_string().len() > 32 * 1024
    {
        return Err(invalid());
    }
    walk(schema, 0, &mut 0)
}

fn valid_type(value: &Value) -> bool {
    let known = |name: &str| {
        [
            "object", "array", "string", "integer", "number", "boolean", "null",
        ]
        .contains(&name)
    };
    value.as_str().is_some_and(known)
        || value.as_array().is_some_and(|types| {
            !types.is_empty()
                && types.len() <= 7
                && types.iter().all(|value| value.as_str().is_some_and(known))
                && types.iter().collect::<HashSet<_>>().len() == types.len()
        })
}

fn strings(value: &Value) -> bool {
    value.as_array().is_some_and(|items| {
        items.len() <= 64
            && items.iter().all(|item| {
                item.as_str().is_some_and(|text| {
                    !text.is_empty() && text.len() <= 160 && !text.chars().any(char::is_control)
                })
            })
            && items.iter().collect::<HashSet<_>>().len() == items.len()
    })
}

fn pattern(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() <= 512
            && regex::RegexBuilder::new(text)
                .size_limit(1024 * 1024)
                .nest_limit(64)
                .build()
                .is_ok()
    })
}

fn children(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), ServiceError> {
    let children = value.as_array().ok_or_else(invalid)?;
    if children.is_empty() || children.len() > 16 {
        return Err(invalid());
    }
    for child in children {
        walk(child, depth + 1, nodes)?;
    }
    Ok(())
}

fn properties(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), ServiceError> {
    let properties = value.as_object().ok_or_else(invalid)?;
    if properties.len() > 64 {
        return Err(invalid());
    }
    for (name, child) in properties {
        if name.is_empty() || name.len() > 160 || name.chars().any(char::is_control) {
            return Err(invalid());
        }
        walk(child, depth + 1, nodes)?;
    }
    Ok(())
}

fn walk(schema: &Value, depth: usize, nodes: &mut usize) -> Result<(), ServiceError> {
    *nodes += 1;
    if depth > 12 || *nodes > 512 {
        return Err(invalid());
    }
    for (name, value) in schema.as_object().ok_or_else(invalid)? {
        let valid = match name.as_str() {
            "type" => valid_type(value),
            "title" | "description" | "$schema" => value.as_str().is_some_and(|s| s.len() <= 2_000),
            "default" | "const" => true,
            "enum" => value
                .as_array()
                .is_some_and(|items| !items.is_empty() && items.len() <= 64),
            "required" => strings(value),
            "uniqueItems" => value.is_boolean(),
            "minLength" | "maxLength" | "minItems" | "maxItems" | "minProperties"
            | "maxProperties" => value.as_u64().is_some(),
            "minimum" | "maximum" => value.as_f64().is_some_and(f64::is_finite),
            "pattern" => pattern(value),
            "format" => value
                .as_str()
                .is_some_and(|s| ["uuid", "date", "date-time", "uri"].contains(&s)),
            "properties" => {
                properties(value, depth, nodes)?;
                true
            }
            "allOf" | "anyOf" | "oneOf" => {
                children(value, depth, nodes)?;
                true
            }
            "items" | "propertyNames" => {
                walk(value, depth + 1, nodes)?;
                true
            }
            "additionalProperties" => {
                if !value.is_boolean() {
                    walk(value, depth + 1, nodes)?;
                }
                true
            }
            _ => return Err(ServiceError::new("personal_page_schema_unsupported")),
        };
        if !valid {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(super) fn validate_action(schema: &Value) -> Result<(), ServiceError> {
    fn forbidden(value: &Value) -> bool {
        match value {
            Value::Object(object) => object.iter().any(|(key, child)| {
                let normalized: String = key
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .flat_map(char::to_lowercase)
                    .collect();
                [
                    "selector",
                    "cssselector",
                    "xpath",
                    "javascript",
                    "script",
                    "coordinates",
                    "screenx",
                    "screeny",
                ]
                .contains(&normalized.as_str())
                    || forbidden(child)
            }),
            Value::Array(items) => items.iter().any(forbidden),
            _ => false,
        }
    }
    validate(schema)?;
    if forbidden(schema) {
        return Err(invalid());
    }
    Ok(())
}
