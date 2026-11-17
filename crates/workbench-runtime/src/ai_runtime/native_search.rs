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
        self.search_conversations_matching(query, archived, cursor, limit, false)
    }

    pub(super) fn search_assistant_conversations(
        &self,
        query: &str,
        archived: bool,
    ) -> Result<UnifiedSearchGroup, String> {
        self.search_conversations_matching(query, archived, None, 10, true)
    }

    fn search_conversations_matching(
        &self,
        query: &str,
        archived: bool,
        cursor: Option<&str>,
        limit: u32,
        keywords: bool,
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
        let terms: Vec<_> = if keywords {
            needle.split_whitespace().collect()
        } else {
            vec![needle.as_str()]
        };
        let signature = crate::persistent_threads::digest(
            &json!({"profile":profile,"query":needle,"archived":archived,"keywords":keywords}),
        );
        let after = search_cursor_after(cursor, &signature)?;
        let provider = if archived {
            "native-archived-conversations"
        } else {
            "native-conversations"
        };
        let mut visible = conversations
            .iter()
            .filter(|conversation| {
                !super::personal_assistant::is_conversation(conversation)
                    && conversation.archived == archived
                    && conversation.conversation_id > after
            })
            .collect::<Vec<_>>();
        visible.sort_by(|a, b| a.conversation_id.cmp(&b.conversation_id));
        let mut items = Vec::new();
        let mut next = None;
        for conversation in visible {
            let title = conversation.title.to_lowercase();
            let title_matches = terms.iter().all(|term| title.contains(term));
            let mut matches = messages.iter().filter(|message| {
                message.conversation_id == conversation.conversation_id && {
                    let body = message.content.to_lowercase();
                    terms.iter().all(|term| body.contains(term))
                        || (!title_matches
                            && terms
                                .iter()
                                .all(|term| body.contains(term) || title.contains(term)))
                }
            });
            let hit = if keywords {
                matches.max_by_key(|message| message.created_at)
            } else {
                matches.next()
            };
            if hit.is_none() && !title_matches {
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
                summary:hit.map(|message| {
                    let body = message.content.to_lowercase();
                    let term = terms.iter().find(|term| body.contains(**term)).copied().unwrap_or(&needle);
                    snippet(&message.content, term)
                }).unwrap_or_default(),
                match_reason:if hit.is_some(){"正文匹配"}else{"名称匹配"}.into(),
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

fn search_cursor_after(cursor: Option<&str>, signature: &str) -> Result<String, &'static str> {
    const INVALID_CURSOR: &str = "conversation_search_cursor_invalid";
    let Some(cursor) = cursor else {
        return Ok(String::new());
    };
    let value: serde_json::Value = serde_json::from_str(cursor).map_err(|_| INVALID_CURSOR)?;
    if value["signature"] != signature {
        return Err(INVALID_CURSOR);
    }
    value["after"]
        .as_str()
        .map(str::to_owned)
        .ok_or(INVALID_CURSOR)
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
