use super::{AppServerServices, ServiceError, SharedAppServerServices};
use crate::capability_creation::capability_creation_request_fingerprint;
use contexthub_persistence_sqlite::CapabilityCreationDeliveryRecord;
use contexthub_protocol::{
    CapabilityCreationAcceptanceExample, CapabilityCreationCreateParams,
    CapabilityCreationSourceFile,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

const CAPABILITY_CREATOR_ID: &str = "system.capability-creator";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Draft {
    title: String,
    business_goal: String,
    target_type: String,
    requested_permissions: Vec<String>,
    acceptance_examples: Vec<CapabilityCreationAcceptanceExample>,
    #[serde(default)]
    source_files: Vec<CapabilityCreationSourceFile>,
}

#[derive(Debug, Deserialize, Serialize)]
struct DraftError {
    reason: String,
    detail: String,
}

fn draft_error(reason: &'static str, detail: impl Into<String>) -> DraftError {
    DraftError {
        reason: reason.to_owned(),
        detail: detail.into(),
    }
}

fn fence(line: &str) -> Option<(u8, usize, &str)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let line = &line[indent..];
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let count = line.bytes().take_while(|byte| *byte == marker).count();
    (count >= 3).then(|| (marker, count, line[count..].trim()))
}

fn parse_draft(content: &str) -> Result<Option<Draft>, DraftError> {
    let mut active: Option<(u8, usize, bool, usize)> = None;
    let mut block = None;
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        if let Some((marker, count, info)) = fence(line) {
            if let Some((opening, length, target, body_start)) = active {
                if marker == opening && count >= length && info.is_empty() {
                    if target {
                        if block.is_some() {
                            return Err(draft_error(
                                "capability_creation_multiple_deliveries",
                                "一次回复只能创建一个能力项目。",
                            ));
                        }
                        block = Some((body_start, offset));
                    }
                    active = None;
                }
            } else {
                active = Some((
                    marker,
                    count,
                    info.eq_ignore_ascii_case("cilai-capability-project"),
                    offset + line.len(),
                ));
            }
        }
        offset += line.len();
    }
    if active.is_some_and(|(_, _, target, _)| target) {
        return Err(draft_error(
            "capability_creation_delivery_incomplete",
            "能力创建方案未完整输出。",
        ));
    }
    let Some((start, end)) = block else {
        return Ok(None);
    };
    let body = &content[start..end];
    if body.len() > 1024 * 1024 {
        return Err(draft_error(
            "capability_creation_delivery_too_large",
            "能力创建方案超过 1 MiB。",
        ));
    }
    let draft = serde_json::from_str(body).map_err(|error| {
        let reason = if error.is_eof() {
            "capability_creation_delivery_incomplete"
        } else if error.is_data() {
            "capability_creation_delivery_fields_invalid"
        } else {
            "capability_creation_delivery_json_invalid"
        };
        draft_error(
            reason,
            format!(
                "能力创建数据第 {} 行、第 {} 列：{error}",
                error.line(),
                error.column()
            ),
        )
    })?;
    Ok(Some(draft))
}

fn message_content(source: &Value) -> String {
    let content = source["message"]["content"].as_str().unwrap_or_default();
    if let Ok(value) = serde_json::from_str::<Value>(content)
        && value.as_object().is_some_and(|object| object.len() == 4)
        && value["schemaVersion"] == 1
        && value["nonce"].is_string()
        && value["classification"].is_object()
        && let Some(reply) = value["reply"].as_str()
    {
        return reply.to_owned();
    }
    content.to_owned()
}

fn used_capability_creator(source: &Value) -> bool {
    source["skillIds"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|id| id == CAPABILITY_CREATOR_ID))
}

pub(super) fn structured_delivery(content: &str) -> Value {
    json!(parse_draft(content))
}

fn source_draft(source: &Value) -> Result<Option<Draft>, DraftError> {
    if let Some(value) = source.get("capabilityDelivery") {
        return serde_json::from_value(value.clone()).map_err(|_| {
            draft_error(
                "capability_creation_delivery_state_invalid",
                "应用交付记录无法读取。",
            )
        })?;
    }
    parse_draft(&message_content(source))
}

impl AppServerServices {
    fn process_capability_creation_delivery(
        &mut self,
        record: &CapabilityCreationDeliveryRecord,
        now: i64,
    ) -> Result<(), ServiceError> {
        let source: Value = serde_json::from_str(&record.source_json)
            .map_err(|_| ServiceError::new("capability_creation_delivery_state_invalid"))?;
        let mut result = json!({
            "runId": record.run_id,
            "status": "pending",
            "creationProjectId": null,
            "errorReason": null,
            "errorDetail": null,
        });
        let status = if !used_capability_creator(&source)
            || !self
                .persistence()?
                .capability_creation_source_run_available(&record.run_id)
                .map_err(persistence_error)?
        {
            "ignored"
        } else {
            match source_draft(&source) {
                Ok(None) => "ignored",
                Err(error) => {
                    result["errorReason"] = Value::String(error.reason.clone());
                    result["errorDetail"] = Value::String(error.detail);
                    "invalid"
                }
                Ok(Some(draft)) => {
                    let mut params = CapabilityCreationCreateParams {
                        title: draft.title,
                        business_goal: draft.business_goal,
                        target_type: draft.target_type,
                        conversation_id: source["conversationId"].as_str().map(str::to_owned),
                        requested_permissions: draft.requested_permissions,
                        acceptance_examples: draft.acceptance_examples,
                        source_files: draft.source_files,
                        client_operation_id: format!("desktop-ai-capability-{}", record.run_id),
                        request_fingerprint: String::new(),
                    };
                    params.request_fingerprint = capability_creation_request_fingerprint(&params);
                    match self.capability_creation_create(&params, now) {
                        Ok(created) => {
                            result["creationProjectId"] =
                                Value::String(created.project.creation_project_id);
                            "created"
                        }
                        Err(error) => {
                            result["errorReason"] = Value::String(error.reason.to_owned());
                            "failed"
                        }
                    }
                }
            }
        };
        result["status"] = Value::String(status.to_owned());
        self.persistence()?
            .finish_capability_creation_delivery(&record.run_id, status, &result)
            .map_err(persistence_error)
    }

    pub(super) fn process_pending_capability_projects(
        &mut self,
        now: i64,
    ) -> Result<(), ServiceError> {
        for run_id in self
            .persistence()?
            .pending_capability_creation_deliveries()
            .map_err(persistence_error)?
        {
            if let Some(record) = self
                .persistence()?
                .capability_creation_delivery(&run_id)
                .map_err(persistence_error)?
            {
                self.process_capability_creation_delivery(&record, now)?;
            }
        }
        Ok(())
    }
}

fn persistence_error(error: contexthub_persistence_sqlite::PersistenceError) -> ServiceError {
    let reason = error
        .reason()
        .unwrap_or("capability_creation_persistence_failed");
    drop(error);
    ServiceError::new(reason)
}

pub fn spawn(
    services: SharedAppServerServices,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            if *shutdown.borrow() {
                break;
            }
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = interval.tick() => {
                    let services = services.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        let mut services = services.lock().map_err(|_| "capability_creation_service_lock_failed")?;
                        services.process_pending_capability_projects(super::unix_epoch_millis().map_err(|_| "capability_creation_clock_failed")?)
                            .map_err(|error| error.reason)
                    }).await;
                    if !matches!(result, Ok(Ok(()))) {
                        eprintln!("capability_creation_worker_failed: {result:?}");
                        break;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_persistence_sqlite::SqlitePersistence;
    use contexthub_protocol::{AiStateReplaceParams, CapabilityCreationListParams};
    use std::fs;
    use std::path::Path;
    use uuid::Uuid;

    fn root() -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("cilai-capability-generation-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn services(root: &Path) -> AppServerServices {
        AppServerServices::with_data_dir(
            Some(SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap()),
            Some(root.to_owned()),
        )
    }

    fn state(content: &str, skill_ids: &Value) -> Value {
        json!({
            "conversations": [{"conversationId":"conversation"}],
            "runs": [{
                "runId":"run-capability",
                "conversationId":"conversation",
                "status":"completed",
                "superseded":false,
                "skillIds":skill_ids,
                "artifacts":[]
            }],
            "messages": [{
                "runId":"run-capability",
                "messageId":"final",
                "role":"assistant",
                "content":content
            }]
        })
    }

    fn content() -> String {
        format!(
            "方案如下。\n\n```cilai-capability-project\n{}\n```",
            json!({
                "title":"本地文件检查",
                "businessGoal":"读取员工选择的目录并返回问题清单",
                "targetType":"local_mcp",
                "requestedPermissions":["personal_file.read"],
                "acceptanceExamples":[{
                    "input":"选择目录",
                    "expected":"返回问题文件，不修改原文件"
                }]
            })
        )
    }

    #[test]
    fn selected_creator_turn_creates_project_once() {
        let root = root();
        let mut server = services(&root);
        let snapshot = state(&content(), &json!([CAPABILITY_CREATOR_ID]));
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: snapshot.clone(),
                },
                1,
            )
            .unwrap();
        server.process_pending_capability_projects(2).unwrap();
        let projects = server
            .capability_creation_list(&CapabilityCreationListParams {}, 2)
            .unwrap()
            .projects;
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].conversation_id.as_deref(), Some("conversation"));
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 1,
                    state: snapshot,
                },
                3,
            )
            .unwrap();
        server.process_pending_capability_projects(4).unwrap();
        assert_eq!(
            server
                .capability_creation_list(&CapabilityCreationListParams {}, 4)
                .unwrap()
                .projects
                .len(),
            1
        );
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unselected_creator_turn_is_ignored() {
        let root = root();
        let mut server = services(&root);
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: state(&content(), &json!([])),
                },
                1,
            )
            .unwrap();
        server.process_pending_capability_projects(2).unwrap();
        assert!(
            server
                .capability_creation_list(&CapabilityCreationListParams {}, 2)
                .unwrap()
                .projects
                .is_empty()
        );
        let delivery = server
            .persistence()
            .unwrap()
            .capability_creation_delivery("run-capability")
            .unwrap()
            .unwrap();
        assert_eq!(delivery.status, "ignored");
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }
}
