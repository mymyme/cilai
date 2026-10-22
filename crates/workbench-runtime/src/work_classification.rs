use serde_json::Value;

/// Only a response bound to this run's nonce is an internal envelope. JSON
/// requested by the user remains ordinary reply content, including a `reply` key.
/// Envelopes copied from earlier turns are hidden as well (see `stale_workbench_nonce`).
pub(crate) fn visible_reply(
    output: &str,
    nonce: &str,
    partial: bool,
) -> Result<Option<String>, String> {
    visible_reply_with(output, nonce, partial, true)
}

/// 历史正文按保存时的规则还原：只识别本轮 nonce 的信封，用于比对既有内容摘要。
pub(crate) fn saved_visible_reply(output: &str, nonce: &str) -> Result<Option<String>, String> {
    visible_reply_with(output, nonce, false, false)
}

fn visible_reply_with(
    output: &str,
    nonce: &str,
    partial: bool,
    hide_stale: bool,
) -> Result<Option<String>, String> {
    let visible = strip_appended_metadata(output, nonce, partial, hide_stale);
    let trimmed = visible.trim_start();
    let source = trimmed
        .strip_prefix("```json")
        .map_or(trimmed, str::trim_start);
    if metadata_header(source, nonce, partial, hide_stale) {
        if partial {
            Ok(reply_prefix(visible))
        } else {
            reply(visible).map(Some)
        }
    } else if partial || !visible.trim().is_empty() {
        Ok(Some(visible.to_owned()))
    } else {
        Err("ai_response_envelope_content_invalid".into())
    }
}

/// 按保存时的规则去掉附加在正文后的本轮信封；历史比对依赖其结果不随显示规则变化。
pub(crate) fn without_appended_metadata<'a>(
    output: &'a str,
    nonce: &str,
    partial: bool,
) -> &'a str {
    strip_appended_metadata(output, nonce, partial, false)
}

fn strip_appended_metadata<'a>(
    output: &'a str,
    nonce: &str,
    partial: bool,
    hide_stale: bool,
) -> &'a str {
    let trimmed = output.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with("```json") {
        return output;
    }
    let mut scan_from = 0;
    for (offset, separator) in output.match_indices(char::is_whitespace) {
        if offset < scan_from {
            continue;
        }
        let suffix = output[offset + separator.len()..].trim_start();
        let json = suffix
            .strip_prefix("```json")
            .map_or(suffix, str::trim_start);
        if metadata_header(json, nonce, partial, hide_stale) {
            return output[..offset].trim_end();
        }
        // Skip complete user data so nested objects or quoted examples cannot
        // be mistaken for a separate internal envelope after an inline space.
        if json.starts_with(['{', '[']) {
            let mut values = serde_json::Deserializer::from_str(json).into_iter::<Value>();
            if matches!(values.next(), Some(Ok(_))) {
                scan_from = output.len() - json.len() + values.byte_offset();
            }
        }
    }
    output
}

fn metadata_header(source: &str, nonce: &str, partial: bool, hide_stale: bool) -> bool {
    let Some(mut rest) = source.strip_prefix('{') else {
        return false;
    };
    let mut version = None;
    let mut actual_nonce = None;
    loop {
        rest = rest.trim_start();
        let mut keys = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        let key = match keys.next() {
            Some(Ok(key)) => key,
            Some(Err(error)) => return partial && error.is_eof(),
            None => return partial,
        };
        if !matches!(key.as_str(), "schemaVersion" | "nonce" | "reply") {
            return false;
        }
        rest = rest[keys.byte_offset()..].trim_start();
        let Some(value_source) = rest.strip_prefix(':') else {
            return partial && rest.is_empty();
        };
        let mut values = serde_json::Deserializer::from_str(value_source).into_iter::<Value>();
        let value = match values.next() {
            Some(Ok(value)) => value,
            Some(Err(error)) => return partial && error.is_eof(),
            None => return partial,
        };
        match key.as_str() {
            "schemaVersion" => {
                if value.as_u64() != Some(1) {
                    return false;
                }
                version = Some(1);
            }
            "nonce" => {
                if !value.as_str().is_some_and(|value| {
                    value == nonce || (hide_stale && stale_workbench_nonce(value))
                }) {
                    return false;
                }
                actual_nonce = Some(());
            }
            _ => {}
        }
        if version.is_some() && actual_nonce.is_some() {
            return true;
        }
        rest = value_source[values.byte_offset()..].trim_start();
        let Some(next) = rest.strip_prefix(',') else {
            return partial && rest.is_empty();
        };
        rest = next;
    }
}

/// 模型可能照抄历史回复里的工作台信封；`classification_<uuid>` 是工作台生成的格式，即使不属于
/// 本轮也只能是内部信封，按信封隐藏其字段。本轮归类校验仍只认当前 nonce。
fn stale_workbench_nonce(value: &str) -> bool {
    value
        .strip_prefix("classification_")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

pub(crate) fn reply(output: &str) -> Result<String, String> {
    extract_reply(output, false)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "ai_response_envelope_content_invalid".to_owned())
}

pub(crate) fn reply_prefix(output: &str) -> Option<String> {
    extract_reply(output, true)
}

fn extract_reply(output: &str, partial: bool) -> Option<String> {
    let trimmed = output.trim_start();
    let source = if let Some(fenced) = trimmed.strip_prefix("```json") {
        fenced.trim_start()
    } else if trimmed.starts_with('{') {
        trimmed
    } else if partial && "```json".starts_with(trimmed) {
        return None;
    } else {
        return Some(output.to_owned());
    };
    let mut rest = source.strip_prefix('{')?;
    loop {
        rest = rest.trim_start();
        let mut key = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
        let name = key.next()?.ok()?;
        let offset = key.byte_offset();
        rest = rest
            .get(offset..)?
            .trim_start()
            .strip_prefix(':')?
            .trim_start();
        if name.as_str() == Some("reply") {
            return decode_string(rest, partial);
        }
        let mut value = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
        value.next()?.ok()?;
        rest = rest
            .get(value.byte_offset()..)?
            .trim_start()
            .strip_prefix(',')?;
    }
}

fn decode_string(source: &str, partial: bool) -> Option<String> {
    let content = source.strip_prefix('"')?;
    let mut escaped = false;
    for (offset, character) in content.char_indices() {
        if character == '"' && !escaped {
            return serde_json::from_str(&source[..offset + 2]).ok();
        }
        escaped = character == '\\' && !escaped;
    }
    if !partial {
        return None;
    }
    let mut end = source.len();
    for _ in 0..13 {
        if let Ok(value) = serde_json::from_str::<String>(&format!("{}\"", &source[..end])) {
            return Some(value);
        }
        end = source[..end].char_indices().next_back()?.0;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{reply, reply_prefix, without_appended_metadata};

    #[test]
    fn envelopes_copied_from_earlier_turns_are_hidden_from_the_reply() {
        let stale = r#"{"schemaVersion":1,"nonce":"classification_1dfe9531-1ca1-4e98-b11b-ffa70e0473ec","reply":"旧轮正文","decision":"create"}"#;
        assert_eq!(
            super::visible_reply(
                &format!("已生成热力图。\n\n{stale}"),
                "classification_current",
                false
            )
            .unwrap(),
            Some("已生成热力图。".into())
        );
        assert_eq!(
            super::visible_reply(stale, "classification_current", false).unwrap(),
            Some("旧轮正文".into())
        );
        // 只有工作台生成格式的标识才按内部信封处理，用户要求的同形 JSON 仍原样显示。
        let user_json = r#"{"schemaVersion":1,"nonce":"classification_demo","reply":"示例"}"#;
        assert_eq!(
            super::visible_reply(user_json, "classification_current", false).unwrap(),
            Some(user_json.into())
        );
    }

    #[test]
    fn ordinary_json_is_not_an_internal_reply_envelope() {
        for raw in [
            r#"{"hello":"world"}"#,
            r#"{"reply":"user data","nonce":"example"}"#,
            "```json\n{\"status\":\"ok\"}\n```",
            "{ incomplete JSON requested by the user",
        ] {
            assert_eq!(
                super::visible_reply(raw, "current", false).unwrap(),
                Some(raw.into())
            );
        }
        let raw =
            r#"{"schemaVersion":1,"nonce":"current","reply":"正文已完成","decision":"invalid"}"#;
        assert_eq!(
            super::visible_reply(raw, "current", false).unwrap(),
            Some("正文已完成".into())
        );
        let malformed =
            r#"{"schemaVersion":1,"nonce":"current","reply":"正文已完成","decision":invalid}"#;
        assert_eq!(
            super::visible_reply(malformed, "current", false).unwrap(),
            Some("正文已完成".into())
        );
        let missing = r#"{"schemaVersion":1,"nonce":"current","decision":"create"}"#;
        assert!(super::visible_reply(missing, "current", false).is_err());
    }

    #[test]
    fn appended_internal_envelope_never_enters_visible_reply() {
        let body = "图片已生成，文件见 [图片](generated.png)。";
        for marker in ["\n\n", "\n```json\n", "\n  ", " ", "\t", "\u{3000}"] {
            let raw = format!(
                "{body}{marker}{}",
                r#"{"schemaVersion":1,"nonce":"classification-current","reply":"图片已生成","decision":"keep"}"#
            );
            assert_eq!(
                without_appended_metadata(&raw, "classification-current", false),
                body
            );
            for end in body.len()..raw.len() {
                if raw.is_char_boundary(end) {
                    let visible =
                        without_appended_metadata(&raw[..end], "classification-current", true);
                    assert!(!visible.contains("schemaVersion"), "{visible}");
                    assert!(!visible.contains("classification-current"), "{visible}");
                }
            }
        }
        let user_json = format!(
            "{body}\n{}",
            r#"{"schemaVersion":1,"nonce":"user-example","reply":"示例"}"#
        );
        assert_eq!(
            without_appended_metadata(&user_json, "classification-current", false),
            user_json
        );
        let sample = format!(
            "{body}\n{}",
            r#"{"reply":"其中提到 nonce","data":{"nonce":"classification-current"}}"#
        );
        assert_eq!(
            without_appended_metadata(&sample, "classification-current", false),
            sample
        );
    }

    #[test]
    fn inline_json_examples_keep_their_original_content() {
        for example in [
            r#"{"schemaVersion":1,"nonce":"user-example","reply":"示例"}"#,
            r#"{"schemaVersion":2,"nonce":"classification-current","reply":"示例"}"#,
            r#"{"reply":"示例","data":{"schemaVersion":1,"nonce":"classification-current"}}"#,
            r#"{"data": {"schemaVersion":1,"nonce":"classification-current","reply":"示例"}}"#,
            r#"[ {"schemaVersion":1,"nonce":"classification-current","reply":"示例"} ]"#,
            r#"{"schemaVersion":1,"nonce":"classification-current-prefix","reply":"示例"}"#,
        ] {
            let raw = format!("数据示例： {example}");
            assert_eq!(
                without_appended_metadata(&raw, "classification-current", false),
                raw
            );
        }
    }

    #[test]
    fn streaming_projects_only_reply_and_decodes_split_escapes() {
        let raw = r#"{"schemaVersion":1,"nonce":"secret","reply":"你好\n\uD842\uDFB7","decision":"keep"}"#;
        assert_eq!(reply(raw).unwrap(), "你好\n𠮷");
        for end in 0..raw.len() {
            if raw.is_char_boundary(end)
                && let Some(text) = reply_prefix(&raw[..end])
            {
                assert!("你好\n𠮷".starts_with(&text), "{text}");
            }
        }
    }

    #[test]
    fn fenced_envelope_preserves_reply_without_exposing_metadata() {
        let body = "命令已执行。\n```powershell\nWrite-Output '你好𠮷'\n```";
        let envelope = format!(
            r#"{{"schemaVersion":1,"nonce":"classification-private","reply":{},"decision":"keep"}}"#,
            serde_json::to_string(body).unwrap()
        );
        for newline in ["\n", "\r\n"] {
            let raw = format!("  ```json{newline}{envelope}{newline}```");
            let visible = without_appended_metadata(&raw, "classification-private", false);
            assert_eq!(visible, raw);
            assert_eq!(reply(visible).unwrap(), body);
            for end in 0..=raw.len() {
                if raw.is_char_boundary(end) {
                    let visible =
                        without_appended_metadata(&raw[..end], "classification-private", true);
                    if let Some(text) = reply_prefix(visible) {
                        assert!(body.starts_with(&text), "{text}");
                    }
                }
            }
        }
        assert_eq!(
            reply("```json\n{\"reply\":\"命令已执行\",\"decision\":").unwrap(),
            "命令已执行"
        );
        for raw in [
            "```json\n{\"reply\":\"尚未完成",
            "```json\n{\"reply\":\"  \"}\n```",
            "```json\n{\"nonce\":\"classification-private\"}\n```",
        ] {
            assert!(reply(raw).is_err());
        }
    }

    #[test]
    fn completed_reply_survives_invalid_classification_without_publishing_metadata() {
        assert_eq!(
            reply(r#"{"reply":"图片已保存","decision":"invalid"}"#).unwrap(),
            "图片已保存"
        );
        assert_eq!(
            reply(r#"{"reply":"图片已保存","decision":"#).unwrap(),
            "图片已保存"
        );
        assert!(reply(r#"{"reply":"尚未完成"#).is_err());
        assert!(reply(r#"{"reply":"  "}"#).is_err());
        assert_eq!(
            reply("图片已保存，归类字段缺失").unwrap(),
            "图片已保存，归类字段缺失"
        );
        assert_eq!(
            reply_prefix("正在生成图片"),
            Some("正在生成图片".to_owned())
        );
        assert!(reply(r#"{"nonce":"secret","decision":"create"}"#).is_err());
        assert!(reply("```json\n{\"nonce\":\"secret\"}\n```").is_err());
        assert!(reply_prefix(r#"{"nonce":"reply","decision":"create"}"#).is_none());
    }
}
