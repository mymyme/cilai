use serde_json::Value;

pub(super) fn introduces_replacement(previous: &Value, next: &Value) -> bool {
    match (previous, next) {
        (Value::String(before), Value::String(after)) => {
            after.matches('\u{fffd}').count() > before.matches('\u{fffd}').count()
        }
        (Value::Object(before), Value::Object(after)) => before.iter().any(|(key, value)| {
            after
                .get(key)
                .is_some_and(|next| introduces_replacement(value, next))
        }),
        (Value::Array(before), Value::Array(after)) => {
            for key in [
                "artifactId",
                "eventId",
                "messageId",
                "runId",
                "conversationId",
                "id",
            ] {
                if !before.is_empty() && before.iter().chain(after).all(|v| v[key].is_string()) {
                    return before.iter().any(|old| {
                        after
                            .iter()
                            .find(|new| new[key] == old[key])
                            .is_some_and(|new| introduces_replacement(old, new))
                    });
                }
            }
            before
                .iter()
                .zip(after)
                .any(|(a, b)| introduces_replacement(a, b))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_lossy_rewrite_without_rejecting_existing_damage_or_repair() {
        let clean = json!({"artifacts":[{"path":"测试技能/蓝图.png"}],"output":"原始回答"});
        let damaged = json!({"artifacts":[{"path":"测试��能/蓝图.png"}],"output":"原始回答"});
        assert!(introduces_replacement(&clean, &damaged));
        assert!(!introduces_replacement(&damaged, &damaged));
        assert!(!introduces_replacement(&damaged, &clean));
        assert!(!introduces_replacement(
            &clean,
            &json!({"output":"正常更新"})
        ));
    }

    #[test]
    fn rejects_every_lossy_utf8_split_and_preserves_reordered_artifact_identity() {
        let original = "测试技能/图片-蓝色圆形.png";
        for offset in 1..original.len() {
            let bytes = original.as_bytes();
            let decoded = format!(
                "{}{}",
                String::from_utf8_lossy(&bytes[..offset]),
                String::from_utf8_lossy(&bytes[offset..])
            );
            assert_eq!(
                introduces_replacement(&json!(original), &json!(decoded)),
                !original.is_char_boundary(offset)
            );
        }
        let before =
            json!([{"artifactId":"a","path":"原图.png"},{"artifactId":"b","path":"历史��.png"}]);
        let reordered =
            json!([{"artifactId":"b","path":"历史��.png"},{"artifactId":"a","path":"原图.png"}]);
        assert!(!introduces_replacement(&before, &reordered));
        let damaged =
            json!([{"artifactId":"b","path":"历史��.png"},{"artifactId":"a","path":"原��.png"}]);
        assert!(introduces_replacement(&before, &damaged));
    }
}
