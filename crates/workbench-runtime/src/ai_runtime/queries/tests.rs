use super::{AiRuntimeQuery, AiRuntimeQueryScope, output_preview};
use crate::ai_runtime::{AiRuntimeManager, lock};
use serde_json::json;
use std::path::PathBuf;
use std::time::Instant;

fn request(scope: AiRuntimeQueryScope, selector: Option<(&str, &str)>) -> AiRuntimeQuery {
    let mut value = json!({"scope": scope});
    if let Some((key, id)) = selector {
        value[key] = json!(id);
    }
    serde_json::from_value(value).unwrap()
}

fn manager() -> AiRuntimeManager {
    let manager = AiRuntimeManager::new();
    {
        let mut state = lock(&manager.inner);
        state.data_dir = Some(PathBuf::from("/query-fixture-not-written"));
        state.state_revision = 17;
        for (id, project, archived) in [("a", "p1", false), ("b", "p2", false), ("c", "p1", true)] {
            state.persisted.conversations.push(serde_json::from_value(json!({
                "conversationId":id,"title":id,"conversationType":"project","mode":"discuss",
                "context":{"workspaceRoot":"/query-fixture-not-written","projectId":project},
                "archived":archived,"createdAt":1,"updatedAt":2,
                "workClassification":{"runId":format!("{id}1"),"taskId":"task","taskTitle":"任务",
                "conversationStatus":"active","stage":"in_progress","reason":"关联任务","confidence":1.0}
            })).unwrap());
            for suffix in [1, 2] {
                let run_id = format!("{id}{suffix}");
                let input_id = format!("input-{run_id}");
                state.persisted.runs.push(serde_json::from_value(json!({
                    "runId":run_id,"conversationId":id,"agentId":"agent","inputMessageId":input_id,
                    "retryOfRunId":if suffix == 2 { Some(format!("{id}1")) } else { None },
                    "provider":"codex","status":if suffix == 1 { "completed" } else { "failed" },
                    "startedAt":1,"output":"结果".repeat(1000),"modelSessionScope":"private-scope",
                    "automationRuleId":if suffix == 1 { Some("rule") } else { None },
                    "events":[{"sequence":1,"eventType":"step","summary":"event-detail","createdAt":1}],
                    "logTail":["private-log"],"artifacts":[{"artifactId":"artifact","artifactType":"file",
                    "label":"产物","path":"/result.md","version":{"digest":"digest","sizeBytes":1,
                    "createdAt":1,"snapshotPath":"/private/snapshot"}}]
                })).unwrap());
                for role in ["user", "assistant"] {
                    state.persisted.messages.push(serde_json::from_value(json!({
                        "messageId":if role == "user" { input_id.clone() } else { format!("reply-{run_id}") },
                        "conversationId":id,"role":role,"content":format!("{role}-{run_id}"),
                        "runId":run_id,"createdAt":1,
                        "attachments":[{"attachmentId":"attachment","fileName":"file.png","mimeType":"image/png",
                        "mediaType":"image","sizeBytes":1,"localPath":"/private/attachment"}]
                    })).unwrap());
                }
            }
        }
        state.persisted.permission_requests.push(serde_json::from_value(json!({
            "permissionRequestId":"permission","runId":"a2","agentId":"agent","workspaceRoot":"/query-fixture-not-written",
            "directoryScope":"workspace","tools":[],"networkAccess":false,"credentialLabels":[],
            "reason":"请求执行","riskLevel":"low","status":"pending","requestedAt":1,"validUntil":2
        })).unwrap());
    }
    manager
}

#[test]
fn purpose_queries_keep_project_boundaries_and_retry_metadata() {
    let manager = manager();
    let project = manager
        .query(&request(
            AiRuntimeQueryScope::Project,
            Some(("projectId", "p1")),
        ))
        .unwrap();
    assert_eq!(project.state_revision, 17);
    assert_eq!(project.conversations.len(), 2);
    assert_eq!(project.runs.len(), 4);
    assert!(project.messages.is_empty() && project.run_details.is_empty());
    assert!(project.runs.iter().all(|run| run.output_preview.is_none()));
    assert!(project.runs.iter().all(|run| {
        run.artifacts[0]
            .version
            .as_ref()
            .unwrap()
            .snapshot_path
            .is_empty()
    }));
    let incidents = manager
        .query(&request(AiRuntimeQueryScope::Incidents, None))
        .unwrap();
    assert_eq!(incidents.runs.len(), 6);
    assert_eq!(incidents.runs[1].retry_of_run_id.as_deref(), Some("a1"));
    assert_eq!(
        incidents.conversations[0]
            .work_classification
            .as_ref()
            .unwrap()
            .task_id
            .as_deref(),
        Some("task")
    );
    let automations = manager
        .query(&request(AiRuntimeQueryScope::Automations, None))
        .unwrap();
    assert_eq!(automations.runs.len(), 3);
    let archive = manager
        .query(&request(AiRuntimeQueryScope::Archive, None))
        .unwrap();
    assert_eq!(archive.conversations[0].conversation_id, "c");
    assert!(archive.messages.is_empty() && archive.runs.is_empty());
}

#[test]
fn work_query_only_delivers_selected_user_requests_and_bounded_previews() {
    let manager = manager();
    let mut query = request(AiRuntimeQueryScope::Work, Some(("projectId", "p1")));
    query.run_ids = Some(vec!["a2".into(), "b1".into()]);
    let work = manager.query(&query).unwrap();
    assert_eq!(work.runs.len(), 1);
    assert_eq!(work.runs[0].run_id, "a2");
    assert_eq!(
        work.runs[0]
            .output_preview
            .as_ref()
            .unwrap()
            .chars()
            .count(),
        240
    );
    assert_eq!(work.messages.len(), 1);
    assert_eq!(work.messages[0].message_id, "input-a2");
    assert!(work.messages[0].attachments[0].local_path.is_empty());
    assert_eq!(
        output_preview(r#"{"reply":"答复","classification":{"taskId":"task"}}"#),
        "答复"
    );
}

#[test]
fn long_work_histories_are_not_rejected_by_the_number_of_run_ids() {
    let manager = manager();
    let mut query = request(AiRuntimeQueryScope::Work, Some(("projectId", "p1")));
    let mut ids = (0..1001)
        .map(|index| format!("historic-{index}"))
        .collect::<Vec<_>>();
    ids.push("a2".into());
    query.run_ids = Some(ids);
    let work = manager.query(&query).unwrap();
    assert_eq!(work.runs.len(), 1);
    assert_eq!(work.runs[0].run_id, "a2");
    assert_eq!(work.messages[0].message_id, "input-a2");
}

#[test]
fn details_keep_full_payload_and_permissions_without_duplicate_summaries() {
    let manager = manager();
    let details = manager
        .query(&request(
            AiRuntimeQueryScope::Conversation,
            Some(("conversationId", "a")),
        ))
        .unwrap();
    assert_eq!(details.run_details.len(), 2);
    assert_eq!(details.messages.len(), 4);
    assert_eq!(details.permission_requests.len(), 1);
    assert!(details.runs.is_empty());
    assert_eq!(details.run_details[0].events[0].summary, "event-detail");
    assert_eq!(
        details.run_details[0]
            .output
            .as_ref()
            .unwrap()
            .chars()
            .count(),
        2000
    );
    assert!(details.run_details[0].model_session_scope.is_none());
    assert!(details.run_details[0].log_tail.is_empty());
    let run = manager
        .query(&request(AiRuntimeQueryScope::Run, Some(("runId", "a2"))))
        .unwrap();
    assert_eq!(run.run_details.len(), 1);
    assert_eq!(run.messages.len(), 2);
    assert_eq!(run.permission_requests.len(), 1);
}

#[test]
fn invalid_or_missing_targets_never_expand_to_all_history() {
    let manager = manager();
    assert_eq!(
        manager
            .query(&request(AiRuntimeQueryScope::Project, None))
            .unwrap_err(),
        "ai_runtime_query_invalid"
    );
    assert_eq!(
        manager
            .query(&request(
                AiRuntimeQueryScope::Run,
                Some(("runId", "missing"))
            ))
            .unwrap_err(),
        "agent_run_not_found"
    );
    assert_eq!(
        manager
            .query(&request(
                AiRuntimeQueryScope::Conversation,
                Some(("conversationId", "missing"))
            ))
            .unwrap_err(),
        "conversation_not_found"
    );
    let mut query = request(AiRuntimeQueryScope::Archive, None);
    query.project_id = Some("p1".into());
    assert_eq!(
        manager.query(&query).unwrap_err(),
        "ai_runtime_query_invalid"
    );
    lock(&manager.inner).shutting_down = true;
    assert_eq!(
        manager
            .query(&request(AiRuntimeQueryScope::Incidents, None))
            .unwrap_err(),
        "ai_runtime_shutting_down"
    );
}

fn measure<T: serde::Serialize>(name: &str, snapshot: &T, elapsed: std::time::Duration) -> usize {
    let start = Instant::now();
    let bytes = serde_json::to_vec(snapshot).unwrap().len();
    println!(
        "query_benchmark scope={name} projection_us={} serialization_us={} bytes={bytes}",
        elapsed.as_micros(),
        start.elapsed().as_micros()
    );
    bytes
}

#[test]
#[ignore = "显式运行一万条历史消息与一万条运行事件的序列化对比"]
fn benchmark_purpose_queries_with_ten_thousand_messages() {
    let manager = manager();
    {
        let mut state = lock(&manager.inner);
        let mut message = state.persisted.messages[0].clone();
        message.content = "历史消息内容".repeat(128);
        state.persisted.messages = (0..10_000)
            .map(|index| {
                let mut value = message.clone();
                value.message_id = format!("message-{index}");
                value.conversation_id = if index % 2 == 0 {
                    "a".into()
                } else {
                    "b".into()
                };
                value
            })
            .collect();
        let mut run = state.persisted.runs[0].clone();
        let mut event = run.events[0].clone();
        event.summary = "运行步骤".repeat(64);
        run.events = (0..50)
            .map(|index| {
                let mut value = event.clone();
                value.sequence = index + 1;
                value
            })
            .collect();
        run.output = Some("回复正文".repeat(512));
        state.persisted.runs = (0..200)
            .map(|index| {
                let mut value = run.clone();
                value.run_id = format!("run-{index}");
                value.conversation_id = if index % 2 == 0 {
                    "a".into()
                } else {
                    "b".into()
                };
                value
            })
            .collect();
    }
    let start = Instant::now();
    let snapshot = manager.live_snapshot();
    let elapsed = start.elapsed();
    let full_bytes = measure("full", &snapshot, elapsed);
    for query in [
        request(AiRuntimeQueryScope::Project, Some(("projectId", "p1"))),
        request(AiRuntimeQueryScope::Automations, None),
    ] {
        let start = Instant::now();
        let result = manager.query(&query).unwrap();
        let elapsed = start.elapsed();
        let bytes = measure(
            if query.scope == AiRuntimeQueryScope::Project {
                "project"
            } else {
                "automations"
            },
            &result,
            elapsed,
        );
        assert!(bytes * 20 < full_bytes);
    }
}
