use crate::{AppServerServices, ServiceError, global_search};
use chrono::{DateTime, SecondsFormat};
use contexthub_protocol::{
    GlobalSearchParams, UnifiedSearchGroup, UnifiedSearchItem, UnifiedSearchLocalTarget,
    UnifiedSearchParams, UnifiedSearchPartialFailure, UnifiedSearchResult,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

const LOCAL_PROVIDER_ID: &str = "com.cilai.workbench.local";

type CursorEntry = (String, u32, global_search::SearchPosition, Instant);

#[derive(Clone, Default)]
pub(super) struct LocalSearchCursors {
    entries: Arc<Mutex<HashMap<String, CursorEntry>>>,
}
impl LocalSearchCursors {
    fn page(
        &mut self,
        reader: &mut contexthub_persistence_sqlite::GlobalSearchReader,
        params: &UnifiedSearchParams,
    ) -> Result<UnifiedSearchGroup, String> {
        let query = params.query.trim();
        let after = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "unified_search_cursor_unavailable".to_owned())?;
            entries.retain(|_, entry| entry.3.elapsed() < Duration::from_mins(20));
            params
                .cursors
                .iter()
                .find(|cursor| cursor.provider_id == LOCAL_PROVIDER_ID)
                .map(|cursor| {
                    entries
                        .get(&cursor.value)
                        .filter(|entry| entry.0 == query && entry.1 == params.limit)
                        .map(|entry| entry.2.clone())
                        .ok_or_else(|| "unified_search_cursor_expired".to_owned())
                })
                .transpose()?
        };
        let (page, next) = global_search::query_reader_page(
            reader,
            &GlobalSearchParams {
                query: query.to_owned(),
                limit: params.limit,
            },
            after,
        )
        .map_err(|error| error.reason().unwrap_or("local_search_failed").to_owned())?;
        let mut group = local_group(page.items);
        if let Some(after) = next {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "unified_search_cursor_unavailable".to_owned())?;
            if entries.len() >= 256
                && let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.3)
                    .map(|(key, _)| key.clone())
            {
                entries.remove(&oldest);
            }
            let cursor = uuid::Uuid::new_v4().to_string();
            entries.insert(
                cursor.clone(),
                (query.to_owned(), params.limit, after, Instant::now()),
            );
            group.next_cursor = Some(cursor);
        }
        Ok(group)
    }
}

pub(super) struct UnifiedSearchPlan {
    request_id: String,
    local: Result<contexthub_persistence_sqlite::GlobalSearchReader, String>,
    cursors: LocalSearchCursors,
}

impl AppServerServices {
    pub(super) fn prepare_unified_search(
        &mut self,
        params: &UnifiedSearchParams,
    ) -> Result<UnifiedSearchPlan, ServiceError> {
        validate_params(params)?;
        let local = self
            .persistence
            .as_ref()
            .ok_or_else(|| "persistence_unavailable".to_owned())
            .and_then(|persistence| {
                persistence
                    .global_search_reader()
                    .map_err(|error| error.reason().unwrap_or("local_search_failed").to_owned())
            });
        Ok(UnifiedSearchPlan {
            request_id: uuid::Uuid::new_v4().to_string(),
            local,
            cursors: self.search_cursors.clone(),
        })
    }
}

pub(super) fn complete_unified_search(
    mut plan: UnifiedSearchPlan,
    params: &UnifiedSearchParams,
) -> UnifiedSearchResult {
    let mut groups = Vec::new();
    let mut partial_failures = Vec::new();
    let result = plan
        .local
        .and_then(|mut reader| plan.cursors.page(&mut reader, params));
    match result {
        Ok(group) => groups.push(group),
        Err(code) => partial_failures.push(UnifiedSearchPartialFailure {
            group_id: "local".to_owned(),
            provider_id: LOCAL_PROVIDER_ID.to_owned(),
            code,
            message: "本地检索未完成；分页失效时请重新搜索".to_owned(),
        }),
    }
    UnifiedSearchResult {
        request_id: plan.request_id,
        authorization_revision: None,
        groups,
        partial_failures,
    }
}

fn validate_params(params: &UnifiedSearchParams) -> Result<(), ServiceError> {
    let normalized = params.query.trim();
    if normalized.chars().count() < 2
        || normalized.chars().count() > 200
        || !(1..=20).contains(&params.limit)
        || params.scope.is_empty()
    {
        return Err(ServiceError::new("unified_search_params_invalid"));
    }
    if params.scope != ["local"] {
        return Err(ServiceError::new("personal_search_scope_not_supported"));
    }
    if params.cursors.len() > 1
        || params.cursors.iter().any(|cursor| {
            cursor.provider_id != LOCAL_PROVIDER_ID
                || cursor.value.is_empty()
                || cursor.value.len() > 8192
                || cursor.value.chars().any(char::is_control)
        })
    {
        return Err(ServiceError::new("unified_search_cursor_invalid"));
    }
    Ok(())
}

fn local_group(items: Vec<contexthub_protocol::GlobalSearchItem>) -> UnifiedSearchGroup {
    UnifiedSearchGroup {
        group_id: "local".to_owned(),
        provider_id: LOCAL_PROVIDER_ID.to_owned(),
        label: "本地工作".to_owned(),
        kind: "local".to_owned(),
        items: items
            .into_iter()
            .map(|item| UnifiedSearchItem {
                hit_type: "local".to_owned(),
                provider_id: LOCAL_PROVIDER_ID.to_owned(),
                object_type: item.object_type.clone(),
                object_id: item.object_id.clone(),
                title: item.title,
                summary: item.subtitle,
                match_reason: "本地工作事实匹配".to_owned(),
                updated_at: DateTime::<chrono::Utc>::from_timestamp_millis(item.updated_at)
                    .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true)),
                deep_link: None,
                local_target: Some(UnifiedSearchLocalTarget {
                    project_id: item.project_id,
                    workspace_id: item.workspace_id.clone(),
                    phase_id: item.phase_id,
                    deliverable_id: item.deliverable_id,
                    local_file_id: item.local_file_id,
                    relative_path: item.relative_path,
                }),
                source_identity: json!({
                    "providerId": LOCAL_PROVIDER_ID,
                    "workspaceId": item.workspace_id,
                    "objectType": item.object_type,
                    "objectId": item.object_id,
                }),
                business_reference: None,
                knowledge_reference: None,
            })
            .collect(),
        next_cursor: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> UnifiedSearchParams {
        UnifiedSearchParams {
            query: "报告".into(),
            scope: vec!["local".into()],
            limit: 8,
            cursors: Vec::new(),
        }
    }

    #[test]
    fn search_requires_explicit_local_scope_and_local_cursor() {
        assert!(validate_params(&params()).is_ok());
        for scope in [
            vec!["knowledge"],
            vec!["applications"],
            vec!["local", "knowledge"],
            vec!["local", "local"],
        ] {
            let mut request = params();
            request.scope = scope.into_iter().map(str::to_owned).collect();
            let error = AppServerServices::new(None)
                .prepare_unified_search(&request)
                .err()
                .unwrap();
            assert_eq!(error.reason, "personal_search_scope_not_supported");
        }
        let mut request = params();
        request
            .cursors
            .push(contexthub_protocol::UnifiedSearchCursor {
                provider_id: "com.cilai.workbench.knowledge".into(),
                value: "old-cursor".into(),
            });
        assert_eq!(
            validate_params(&request).unwrap_err().reason,
            "unified_search_cursor_invalid"
        );
    }

    #[test]
    fn unavailable_local_storage_is_reported_as_an_incomplete_search() {
        let request = params();
        let plan = AppServerServices::new(None)
            .prepare_unified_search(&request)
            .unwrap();
        let result = complete_unified_search(plan, &request);
        assert!(result.groups.is_empty());
        assert_eq!(result.partial_failures.len(), 1);
        assert_eq!(result.partial_failures[0].code, "persistence_unavailable");
        assert!(result.authorization_revision.is_none());
    }
}
