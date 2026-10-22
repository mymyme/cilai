use super::{PersistenceError, PersistenceErrorKind, database_error};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// 失败运行所在会话里若已有后续运行把同一请求做成了，就把对应事件标为已恢复。
///
/// 只读取有未结事件的会话的运行与消息：重试链和同一请求的判定都限定在同一会话内。
pub(super) fn reconcile(transaction: &Transaction<'_>) -> Result<(), PersistenceError> {
    merge_superseded_classification_incidents(transaction)?;
    let cases = {
        let mut statement = transaction.prepare("SELECT attention_case_id,agent_run_id,project_id,task_id FROM attention_cases WHERE case_type='agent_run_incident' AND status='open'").map_err(database_error("read recoverable incidents"))?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(database_error("query recoverable incidents"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("decode recoverable incidents"))?
    };
    for case in cases {
        reconcile_case(transaction, case)?;
    }
    Ok(())
}

fn merge_superseded_classification_incidents(
    transaction: &Transaction<'_>,
) -> Result<(), PersistenceError> {
    let cases = {
        let mut statement = transaction
            .prepare(
                "SELECT older.attention_case_id,newer.attention_case_id,newer.created_at
                 FROM attention_cases older
                 JOIN ai_business_runs older_run ON older_run.run_id=older.agent_run_id
                 JOIN attention_cases newer
                   ON newer.client_operation_id LIKE 'desktop-classification-incident-%'
                  AND newer.status='open'
                 JOIN ai_business_runs newer_run
                   ON newer_run.run_id=newer.agent_run_id
                  AND newer_run.conversation_id=older_run.conversation_id
                 WHERE older.client_operation_id LIKE 'desktop-classification-incident-%'
                   AND older.status='open'
                   AND (
                     newer.created_at>older.created_at
                     OR (newer.created_at=older.created_at AND newer.attention_case_id>older.attention_case_id)
                   )
                   AND NOT EXISTS (
                     SELECT 1 FROM attention_cases latest
                     JOIN ai_business_runs latest_run ON latest_run.run_id=latest.agent_run_id
                     WHERE latest.client_operation_id LIKE 'desktop-classification-incident-%'
                       AND latest.status='open'
                       AND latest_run.conversation_id=older_run.conversation_id
                       AND (
                         latest.created_at>newer.created_at
                         OR (latest.created_at=newer.created_at AND latest.attention_case_id>newer.attention_case_id)
                       )
                   )",
            )
            .map_err(database_error("prepare classification incident merge"))?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(database_error("query classification incident merge"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("decode classification incident merge"))?
    };
    for (case_id, latest_id, created_at) in cases {
        let operation = format!("classification-incident-merged:{case_id}:{latest_id}");
        let summary =
            format!("同一会话已保留更新的工作归类待处理事项：{latest_id}；历史失败记录保留。");
        transaction.execute(
            "INSERT OR IGNORE INTO attention_case_transitions(transition_id,attention_case_id,from_status,to_status,actor_label,resolution_summary,client_operation_id,request_fingerprint,created_at) VALUES (?1,?2,'open','resolved','系统',?3,?1,?1,?4)",
            params![operation, case_id, summary, created_at],
        ).map_err(database_error("record classification incident merge"))?;
        transaction.execute(
            "UPDATE attention_cases SET status='resolved',resolution_summary=?2,resolved_at=?3,updated_at=?3,state_revision=state_revision+1 WHERE attention_case_id=?1 AND status='open'",
            params![case_id, summary, created_at],
        ).map_err(database_error("merge classification incident"))?;
        let payload = serde_json::json!({"attentionCaseId":case_id,"supersededByAttentionCaseId":latest_id,"status":"resolved"});
        transaction.execute(
            "INSERT OR IGNORE INTO durable_events(event_id,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at) VALUES (?1,'attention_case.resolved','attention_case',?2,?3,?1,?4,?4)",
            params![operation, case_id, payload.to_string(), created_at],
        ).map_err(database_error("publish classification incident merge"))?;
    }
    Ok(())
}

fn read_json_rows(
    transaction: &Transaction<'_>,
    sql: &str,
    key: &str,
) -> Result<Vec<Value>, PersistenceError> {
    let mut statement = transaction
        .prepare(sql)
        .map_err(database_error("prepare incident recovery facts"))?;
    let rows = statement
        .query_map([key], |row| row.get::<_, String>(0))
        .map_err(database_error("read incident recovery facts"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("decode incident recovery facts"))?;
    rows.iter()
        .map(|row| {
            serde_json::from_str(row).map_err(|_| {
                PersistenceError::with_reason(
                    PersistenceErrorKind::Configuration,
                    "ai_state_invalid",
                    "Invalid AI state",
                )
            })
        })
        .collect()
}

fn reconcile_case(
    transaction: &Transaction<'_>,
    (case_id, failed_id, project_id, task_id): (String, String, String, Option<String>),
) -> Result<(), PersistenceError> {
    let conversation_id: Option<Option<String>> = transaction
        .query_row(
            "SELECT conversation_id FROM ai_business_runs WHERE run_id=?1",
            [&failed_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error("read failed run conversation"))?;
    let Some(Some(conversation_id)) = conversation_id else {
        return Ok(());
    };
    let runs = read_json_rows(
        transaction,
        "SELECT run_json FROM ai_business_runs WHERE conversation_id=?1 ORDER BY position,run_id",
        &conversation_id,
    )?;
    let conversations = read_json_rows(
        transaction,
        "SELECT conversation_json FROM ai_business_conversations WHERE conversation_id=?1",
        &conversation_id,
    )?;
    let messages = read_json_rows(
        transaction,
        "SELECT message_json FROM ai_business_messages WHERE conversation_id=?1 ORDER BY position,message_id",
        &conversation_id,
    )?;
    let messages = Some(&messages);
    let by_id: HashMap<_, _> = runs
        .iter()
        .filter_map(|run| Some((run.get("runId")?.as_str()?, run)))
        .collect();
    let Some(failed) = by_id.get(failed_id.as_str()) else {
        return Ok(());
    };
    let failed_classification = failed.get("status").and_then(Value::as_str) == Some("completed")
        && failed.get("classificationStatus").and_then(Value::as_str) == Some("failed")
        && task_id.is_none();
    if !failed_classification
        && !matches!(
            failed.get("status").and_then(Value::as_str),
            Some("failed" | "terminated" | "interrupted")
        )
    {
        return Ok(());
    }
    let Some(conversation_id) = failed.get("conversationId").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(conversation) = conversations
        .iter()
        .find(|conversation| conversation["conversationId"] == conversation_id)
    else {
        return Ok(());
    };
    if !recovery_context_matches(
        conversation,
        &project_id,
        task_id.as_deref(),
        failed_classification,
    ) {
        return Ok(());
    }
    let classified_runs = if failed_classification {
        classified_run_ids(transaction, conversation_id, &project_id)?
    } else {
        HashSet::new()
    };
    let recovered = runs
        .iter()
        .filter(|run| {
            run["status"] == "completed"
                && (!failed_classification
                    || run.get("classificationStatus").and_then(Value::as_str) == Some("applied"))
                && run["conversationId"] == conversation_id
                && run
                    .get("startedAt")
                    .and_then(Value::as_i64)
                    .zip(failed.get("completedAt").and_then(Value::as_i64))
                    .is_some_and(|(start, end)| start >= end)
                && if failed_classification {
                    run.get("runId")
                        .and_then(Value::as_str)
                        .is_some_and(|id| classified_runs.contains(id))
                        && run.get("superseded").and_then(Value::as_bool) != Some(true)
                } else {
                    retries(run, &failed_id, &by_id) || same_request(run, failed, messages)
                }
        })
        .min_by_key(|run| run.get("completedAt").and_then(Value::as_i64));
    let Some(recovered) = recovered else {
        return Ok(());
    };
    record_recovery(
        transaction,
        &case_id,
        &failed_id,
        recovered,
        failed_classification,
    )
}

fn recovery_context_matches(
    conversation: &Value,
    project_id: &str,
    task_id: Option<&str>,
    failed_classification: bool,
) -> bool {
    conversation
        .pointer("/context/projectId")
        .and_then(Value::as_str)
        == Some(project_id)
        && (failed_classification
            || conversation
                .pointer("/context/taskId")
                .and_then(Value::as_str)
                == task_id)
}

fn classified_run_ids(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    project_id: &str,
) -> Result<HashSet<String>, PersistenceError> {
    let mut statement = transaction
        .prepare("SELECT run_id FROM conversation_work_classifications WHERE conversation_id=?1 AND project_id=?2")
        .map_err(database_error("prepare applied classification recovery"))?;
    statement
        .query_map(params![conversation_id, project_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(database_error("query applied classification recovery"))?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(database_error("decode applied classification recovery"))
}

fn record_recovery(
    transaction: &Transaction<'_>,
    case_id: &str,
    failed_id: &str,
    recovered: &Value,
    failed_classification: bool,
) -> Result<(), PersistenceError> {
    let Some(completed_at) = recovered.get("completedAt").and_then(Value::as_i64) else {
        return Ok(());
    };
    let Some(recovery_id) = recovered.get("runId").and_then(Value::as_str) else {
        return Ok(());
    };
    let operation = format!("incident-recovered:{case_id}:{recovery_id}");
    let summary = if failed_classification {
        format!("后续回合已成功保存工作归类：{recovery_id}；历史失败记录保留。")
    } else {
        format!("原请求已在后续运行中成功完成：{recovery_id}；历史失败记录保留。")
    };
    transaction.execute("INSERT INTO attention_case_transitions(transition_id,attention_case_id,from_status,to_status,actor_label,resolution_summary,client_operation_id,request_fingerprint,created_at) VALUES (?1,?2,'open','resolved','系统',?3,?1,?1,?4)", params![operation,case_id,summary,completed_at]).map_err(database_error("record incident recovery"))?;
    transaction.execute("UPDATE attention_cases SET status='resolved',resolution_summary=?2,resolved_at=?3,updated_at=?3,state_revision=state_revision+1 WHERE attention_case_id=?1 AND status='open'", params![case_id,summary,completed_at]).map_err(database_error("resolve recovered incident"))?;
    let payload = serde_json::json!({"attentionCaseId":case_id,"failedRunId":failed_id,"recoveredRunId":recovery_id,"status":"resolved"});
    transaction.execute("INSERT INTO durable_events(event_id,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at) VALUES (?1,'attention_case.resolved','attention_case',?2,?3,?1,?4,?4)",params![operation,case_id,payload.to_string(),completed_at]).map_err(database_error("publish incident recovery"))?;
    Ok(())
}

fn retries(run: &Value, failed_id: &str, runs: &HashMap<&str, &Value>) -> bool {
    let mut current = run;
    let mut seen = HashSet::new();
    while let Some(parent) = current.get("retryOfRunId").and_then(Value::as_str) {
        if parent == failed_id {
            return true;
        }
        if !seen.insert(parent) {
            return false;
        }
        let Some(previous) = runs.get(parent) else {
            return false;
        };
        if previous.get("conversationId") != run.get("conversationId") {
            return false;
        }
        current = previous;
    }
    false
}

fn same_request(run: &Value, failed: &Value, messages: Option<&Vec<Value>>) -> bool {
    let Some(messages) = messages else {
        return false;
    };
    let find = |input: &Value| {
        messages.iter().find(|message| {
            input
                .get("inputMessageId")
                .and_then(Value::as_str)
                .is_some()
                && message.get("messageId") == input.get("inputMessageId")
                && message.get("conversationId") == input.get("conversationId")
                && message["role"] == "user"
        })
    };
    let (Some(current), Some(original)) = (find(run), find(failed)) else {
        return false;
    };
    if let Some(digest) = original.get("contentDigest").and_then(Value::as_str) {
        return digest.len() == 64
            && original["contentBytes"]
                .as_u64()
                .is_some_and(|length| length > 0)
            && current.get("contentDigest") == original.get("contentDigest")
            && current.get("attachments") == original.get("attachments");
    }
    original
        .get("content")
        .and_then(Value::as_str)
        .is_some_and(|content| !content.trim().is_empty())
        && current.get("content") == original.get("content")
        && current.get("attachments") == original.get("attachments")
}
