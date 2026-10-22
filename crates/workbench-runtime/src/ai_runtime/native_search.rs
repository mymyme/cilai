use super::AiRuntimeManager;
use contexthub_protocol::{UnifiedSearchGroup, UnifiedSearchItem};
use serde_json::json;

impl AiRuntimeManager {
    /// 检索工作台当前账号可见的正文与标题，包括迁入历史。
    /// # Errors
    /// 搜索参数、分页游标无效或当前账号状态改变时返回错误。
    pub fn search_conversations(
        &self,
        query: &str,
        archived: bool,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<UnifiedSearchGroup, String> {
        let query = query.trim();
        if query.chars().count() < 2 || query.len() > 1000 || !(1..=100).contains(&limit) {
            return Err("conversation_search_invalid".into());
        }
        let (profile, conversations, messages) = {
            let state = super::lock(&self.inner);
            (
                state.data_dir.clone().ok_or("ai_runtime_not_initialized")?,
                state.persisted.conversations.clone(),
                state.persisted.messages.clone(),
            )
        };
        // Search the same visible bodies as the Workbench, including imported history.
        // The native index cannot index Workbench-owned display turns or filter legacy custom sources.
        let needle = query.to_lowercase();
        let signature = crate::persistent_threads::digest(
            &json!({"profile":profile,"query":needle,"archived":archived}),
        );
        let after = if let Some(cursor) = cursor {
            let value: serde_json::Value =
                serde_json::from_str(cursor).map_err(|_| "conversation_search_cursor_invalid")?;
            if value["signature"] != signature {
                return Err("conversation_search_cursor_invalid".into());
            }
            value["after"]
                .as_str()
                .ok_or("conversation_search_cursor_invalid")?
                .to_owned()
        } else {
            String::new()
        };
        let provider = if archived {
            "native-archived-conversations"
        } else {
            "native-conversations"
        };
        let mut visible = conversations
            .iter()
            .filter(|conversation| {
                conversation.archived == archived && conversation.conversation_id > after
            })
            .collect::<Vec<_>>();
        visible.sort_by(|a, b| a.conversation_id.cmp(&b.conversation_id));
        let mut items = Vec::new();
        let mut next = None;
        for conversation in visible {
            let matched = messages.iter().find(|message| {
                message.conversation_id == conversation.conversation_id
                    && message.content.to_lowercase().contains(&needle)
            });
            if matched.is_none() && !conversation.title.to_lowercase().contains(&needle) {
                continue;
            }
            if items.len() == limit as usize {
                next = items.last().map(|item: &UnifiedSearchItem| {
                    json!({"signature":signature,"after":item.object_id}).to_string()
                });
                break;
            }
            items.push(UnifiedSearchItem {
                hit_type:"local".into(),provider_id:provider.into(),object_type:"conversation".into(),
                object_id:conversation.conversation_id.clone(),title:conversation.title.clone(),
                summary:matched.map(|message|snippet(&message.content,&needle)).unwrap_or_default(),
                match_reason:if matched.is_some(){"正文匹配"}else{"名称匹配"}.into(),
                updated_at:None,deep_link:None,local_target:None,
                source_identity:json!({"conversationId":conversation.conversation_id,"archived":archived}),
                business_reference:None,knowledge_reference:None,
            });
        }
        if super::lock(&self.inner).data_dir.as_ref() != Some(&profile) {
            return Err("ai_profile_changed".into());
        }
        Ok(UnifiedSearchGroup {
            group_id: provider.into(),
            provider_id: provider.into(),
            label: if archived {
                "已归档对话"
            } else {
                "对话"
            }
            .into(),
            kind: "conversations".into(),
            items,
            next_cursor: next,
        })
    }
}

fn snippet(text: &str, needle: &str) -> String {
    let lowered = text.to_lowercase();
    let byte = lowered.find(needle).unwrap_or_default();
    let offset = lowered[..byte].chars().count().saturating_sub(50);
    text.chars().skip(offset).take(180).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn chinese_excerpt_is_bounded_and_contains_the_match() {
        let body = format!("{}检索目标{}", "前".repeat(200), "后".repeat(200));
        let excerpt = super::snippet(&body, "检索目标");
        assert!(excerpt.contains("检索目标"));
        assert_eq!(excerpt.chars().count(), 180);
    }
}
