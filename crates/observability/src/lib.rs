#![doc = "默认脱敏的结构化日志、指标和诊断边界。"]

pub const SERVICE_NAME: &str = "contexthub-app-server";

const SENSITIVE_KEYS: &[&str] = &[
    "token",
    "authorization",
    "password",
    "secret",
    "content",
    "body",
];

#[must_use]
pub fn redact_field(key: &str, value: &str) -> String {
    if SENSITIVE_KEYS
        .iter()
        .any(|candidate| key.to_ascii_lowercase().contains(candidate))
    {
        "[REDACTED]".to_owned()
    } else if value.starts_with('/') || value.get(1..3) == Some(":\\") {
        "<absolute-path>".to_owned()
    } else {
        value.to_owned()
    }
}

#[must_use]
pub fn diagnostic_summary(
    instance_id: &str,
    protocol_version: &str,
    database_available: bool,
    local_ipc_available: bool,
    platform_mode: &str,
) -> String {
    format!(
        "{{\"service\":\"{SERVICE_NAME}\",\"instanceId\":\"{}\",\"protocolVersion\":\"{}\",\"databaseAvailable\":{},\"localIpcAvailable\":{},\"platformMode\":\"{}\"}}",
        redact_field("instance_id", instance_id),
        redact_field("protocol_version", protocol_version),
        database_available,
        local_ipc_available,
        redact_field("platform_mode", platform_mode)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_content_and_absolute_paths() {
        assert_eq!(redact_field("accessToken", "abc"), "[REDACTED]");
        assert_eq!(redact_field("fileContent", "private"), "[REDACTED]");
        assert_eq!(
            redact_field("workspace", "/Users/name/private"),
            "<absolute-path>"
        );
        assert_eq!(redact_field("correlation_id", "corr-1"), "corr-1");
    }
}
