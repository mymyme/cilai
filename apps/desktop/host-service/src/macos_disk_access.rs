use serde_json::Value;

fn validate_arguments(args: &Value) -> Result<(), String> {
    if args.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(())
    } else {
        Err("磁盘访问设置不接受额外参数".into())
    }
}

pub fn open_settings(args: &Value) -> Result<Value, String> {
    validate_arguments(args)?;
    contexthub_platform_adapter::macos_disk_access::open_settings()
        .map_err(|error| error.to_string())?;
    Ok(Value::Null)
}

pub fn reveal_application(args: &Value) -> Result<Value, String> {
    validate_arguments(args)?;
    contexthub_platform_adapter::macos_disk_access::reveal_application()
        .map_err(|error| error.to_string())?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_accept_only_empty_objects() {
        validate_arguments(&serde_json::json!({})).unwrap();
        for invalid in [
            Value::Null,
            serde_json::json!([]),
            serde_json::json!({"url":"x-apple.systempreferences:other"}),
            serde_json::json!({"path":"/Applications/Other.app"}),
            serde_json::json!({"action":"grant"}),
        ] {
            assert!(open_settings(&invalid).is_err());
            assert!(reveal_application(&invalid).is_err());
        }
    }
}
