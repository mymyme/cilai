use super::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn invalid(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, PersistenceError> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(|| invalid("application_conversation_request_invalid"))
}
fn decode(raw: &str) -> Result<Value, PersistenceError> {
    serde_json::from_str(raw).map_err(|_| invalid("application_conversation_storage_invalid"))
}
fn active(snapshot: &Value) -> bool {
    matches!(
        snapshot["status"].as_str(),
        Some(
            "queued"
                | "running"
                | "waiting_confirmation"
                | "submitting"
                | "terminating"
                | "recovery_required"
        )
    )
}
fn metadata(snapshot: &Value) -> Result<Value, PersistenceError> {
    let mut result = snapshot.clone();
    let object = result
        .as_object_mut()
        .ok_or_else(|| invalid("application_conversation_request_invalid"))?;
    for key in ["output", "partialOutput", "events", "prompt"] {
        object.remove(key);
    }
    if !matches!(
        result["status"].as_str(),
        Some(
            "queued"
                | "running"
                | "waiting_confirmation"
                | "submitting"
                | "terminating"
                | "recovery_required"
                | "completed"
                | "failed"
                | "cancelled"
        )
    ) {
        return Err(invalid("application_conversation_status_invalid"));
    }
    Ok(result)
}

fn ensure_page_writers_settled(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
) -> Result<(), PersistenceError> {
    let mut statement = tx
        .prepare("SELECT snapshot_json FROM ai_application_messages WHERE session_id=?1")
        .map_err(database_error("read page authorization writers"))?;
    let snapshots = statement
        .query_map([session], |row| row.get::<_, String>(0))
        .map_err(database_error("read page authorization writers"))?;
    for snapshot in snapshots {
        let snapshot =
            decode(&snapshot.map_err(database_error("read page authorization writer"))?)?;
        if active(&snapshot)
            || matches!(
                snapshot
                    .pointer("/writeRequest/approval/executionStatus")
                    .and_then(Value::as_str),
                Some("submitting" | "outcome_unknown")
            )
        {
            return Err(invalid(
                "application_page_authorization_reconciliation_required",
            ));
        }
    }
    Ok(())
}

impl SqlitePersistence {
    fn application_usage_metadata(&self) -> Result<Value, PersistenceError> {
        let mut statement = self.connection.prepare(
            "SELECT json_object('runId',m.run_id,'modelId',json_extract(m.snapshot_json,'$.modelId'),'status',json_extract(m.snapshot_json,'$.status'),'startedAt',json_extract(m.snapshot_json,'$.startedAt'),'completedAt',json_extract(m.snapshot_json,'$.completedAt'),'eventPosition',COALESCE(json_extract(m.snapshot_json,'$.eventPosition'),0),'modelExecution',CASE WHEN json_type(m.snapshot_json,'$.modelExecution')='object' THEN json_object('profileId',json_extract(m.snapshot_json,'$.modelExecution.profileId'),'connectionId',json_extract(m.snapshot_json,'$.modelExecution.connectionId'),'configRevision',json_extract(m.snapshot_json,'$.modelExecution.configRevision'),'authGeneration',json_extract(m.snapshot_json,'$.modelExecution.authGeneration'),'modelId',json_extract(m.snapshot_json,'$.modelExecution.modelId')) ELSE NULL END,'replyUsage',CASE WHEN json_type(m.snapshot_json,'$.replyUsage')='object' THEN json_object('inputTokens',json_extract(m.snapshot_json,'$.replyUsage.inputTokens'),'cachedInputTokens',json_extract(m.snapshot_json,'$.replyUsage.cachedInputTokens'),'outputTokens',json_extract(m.snapshot_json,'$.replyUsage.outputTokens')) ELSE NULL END) FROM ai_application_messages m ORDER BY m.created_at,m.run_id LIMIT 50001",
        ).map_err(database_error("read persisted application usage metadata"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error("query persisted application usage metadata"))?;
        let mut snapshots = Vec::new();
        for row in rows {
            if snapshots.len() == 50_000 {
                return Err(invalid("application_usage_limit_exceeded"));
            }
            snapshots.push(decode(
                &row.map_err(database_error("read application usage row"))?,
            )?);
        }
        Ok(json!({"snapshots":snapshots,"source":"persisted_page_metadata"}))
    }
    fn application_restart_blockers(&self) -> Result<Value, PersistenceError> {
        let mut statement = self.connection.prepare(
            "SELECT m.snapshot_json,EXISTS(SELECT 1 FROM ai_thread_message_links l WHERE (l.conversation_id='application:'||m.session_id AND l.client_message_id=m.client_message_id) OR l.conversation_id='application-task:'||m.run_id) FROM ai_application_messages m",
        ).map_err(database_error("read application restart state"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })
            .map_err(database_error("read application restart state"))?;
        let mut application_active = false;
        let mut write_unknown = false;
        for row in rows {
            let (raw, accepted) = row.map_err(database_error("read application restart state"))?;
            let snapshot = metadata(&decode(&raw)?)?;
            application_active |= active(&snapshot) && (snapshot["status"] != "queued" || accepted);
            write_unknown |= matches!(
                snapshot
                    .pointer("/writeRequest/approval/executionStatus")
                    .and_then(Value::as_str),
                Some("submitting" | "outcome_unknown")
            );
        }
        let native_active: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM ai_conversation_thread_bindings WHERE (conversation_id LIKE 'application:%' OR conversation_id LIKE 'application-task:%') AND active_run_id IS NOT NULL) OR EXISTS(SELECT 1 FROM ai_thread_operations WHERE (conversation_id LIKE 'application:%' OR conversation_id LIKE 'application-task:%') AND state IN ('prepared','dispatched','uncertain'))",
            [], |row| row.get(0),
        ).map_err(database_error("read application restart native operations"))?;
        let mut blockers = Vec::new();
        if application_active || native_active {
            blockers.push("application_model_run_active");
        }
        if write_unknown {
            blockers.push("application_write_outcome_unsettled");
        }
        Ok(json!({"blockers":blockers}))
    }
    /// # Errors
    /// Rejects an unknown instance, conflicting identity, concurrent dispatch or unsafe cleanup.
    #[allow(clippy::too_many_lines)]
    pub fn application_conversation(
        &mut self,
        action: &str,
        payload: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let value = payload;
        if action == "usage" {
            if !value.as_object().is_some_and(serde_json::Map::is_empty) {
                return Err(invalid("application_conversation_request_invalid"));
            }
            return self.application_usage_metadata();
        }
        if action == "restartBlockers" {
            return self.application_restart_blockers();
        }
        if action == "listPending" {
            let mut statement=self.connection.prepare("SELECT session_id,application_id FROM ai_application_sessions WHERE status IN ('closed','cleaning')").map_err(database_error("read pending application cleanup"))?;
            let sessions=statement.query_map([],|r|Ok(json!({"sessionId":r.get::<_,String>(0)?,"applicationId":r.get::<_,String>(1)?}))).map_err(database_error("read pending application cleanup"))?.collect::<Result<Vec<_>,_>>().map_err(database_error("read pending application cleanup"))?;
            return Ok(json!({"sessions":sessions}));
        }
        if action == "listInstance" {
            let instance = text(value, "instanceId")?;
            let application = text(value, "applicationId")?;
            let mut statement=self.connection.prepare("SELECT session_id FROM ai_application_sessions WHERE application_id=?1 AND (instance_id=?2 OR substr(instance_id,1,length(?2)+1)=?2||':') AND status IN ('active','closed','cleaning') AND substr(session_id,1,13)!='model_report_' ORDER BY session_id")
                .map_err(database_error("read instance application sessions"))?;
            let sessions = statement
                .query_map(params![application, instance], |r| r.get::<_, String>(0))
                .map_err(database_error("read instance application sessions"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("read instance application sessions"))?;
            return Ok(json!({"sessionIds":sessions}));
        }
        let mut id = text(value, "sessionId")?;
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("begin application conversation operation"))?;
        let mut scoped_instance = None;
        if action == "openPage" {
            let legacy = text(value, "legacySessionId")?;
            let instance = text(value, "instanceId")?;
            let app = text(value, "applicationId")?;
            let authorization = text(value, "authorizationDigest")?;
            let legacy_authorization = text(value, "legacyAuthorizationDigest")?;
            if legacy != format!("model_session_{:x}", Sha256::digest(instance.as_bytes()))
                || id != format!("model_page_{:x}", Sha256::digest(authorization.as_bytes()))
                || !authorization
                    .strip_prefix("page-v1:")
                    .is_some_and(|digest| {
                        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
                    })
            {
                return Err(invalid("application_page_authorization_invalid"));
            }
            scoped_instance = Some(format!(
                "{instance}:{:x}",
                Sha256::digest(authorization.as_bytes())
            ));
            let current_exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ai_application_sessions WHERE session_id=?1)",
                    [id],
                    |r| r.get(0),
                )
                .map_err(database_error("read current page authorization"))?;
            if !current_exists {
                let mut statement=tx.prepare("SELECT session_id,authorization_digest FROM ai_application_sessions WHERE application_id=?1 AND (instance_id=?2 OR substr(instance_id,1,length(?2)+1)=?2||':')")
                    .map_err(database_error("read previous page scopes"))?;
                let scopes = statement
                    .query_map(params![app, instance], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(database_error("read previous page scopes"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(database_error("read previous page scopes"))?;
                drop(statement);
                for (session, known_authorization) in scopes {
                    if known_authorization != authorization {
                        ensure_page_writers_settled(&tx, &session)?;
                    }
                }
                let previous: Option<(String,String,String,String)> = tx.query_row("SELECT instance_id,application_id,authorization_digest,status FROM ai_application_sessions WHERE session_id=?1",[legacy],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
                    .optional().map_err(database_error("read legacy page authorization"))?;
                if let Some((known_instance, known_app, known_authorization, status)) = previous {
                    if known_instance != instance || known_app != app {
                        return Err(invalid("application_conversation_restore_denied"));
                    }
                    if known_authorization != authorization {
                        ensure_page_writers_settled(&tx, legacy)?;
                    }
                    if known_authorization == authorization
                        || known_authorization == legacy_authorization
                    {
                        if status != "active" {
                            return Err(invalid("application_conversation_restore_denied"));
                        }
                        id = legacy;
                    }
                }
            }
        }
        let existing: Option<(String,String,String,String,i64)> = tx.query_row("SELECT instance_id,application_id,authorization_digest,status,revision FROM ai_application_sessions WHERE session_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(database_error("read application conversation"))?;
        if matches!(action, "open" | "openReport" | "openPage") {
            let instance = if action == "openPage" && id != text(value, "legacySessionId")? {
                scoped_instance
                    .as_deref()
                    .ok_or_else(|| invalid("application_page_authorization_invalid"))?
            } else {
                text(value, "instanceId")?
            };
            let app = text(value, "applicationId")?;
            let authorization = text(value, "authorizationDigest")?;
            let legacy_authorization = if action == "openReport" {
                let legacy = text(value, "legacyAuthorizationDigest")?;
                let is_digest = |digest: &str| {
                    digest.len() == 64
                        && digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                };
                if instance != id
                    || app != "com.cilai.environmental.policy"
                    || !id.strip_prefix("model_report_").is_some_and(is_digest)
                    || !authorization
                        .strip_prefix("report-v1:")
                        .is_some_and(is_digest)
                    || !is_digest(legacy)
                {
                    return Err(invalid("application_report_authorization_invalid"));
                }
                Some(legacy)
            } else if action == "openPage" && id == text(value, "legacySessionId")? {
                Some(text(value, "legacyAuthorizationDigest")?)
            } else {
                None
            };
            if let Some((known_instance, known_app, known_authorization, status, _)) =
                existing.as_ref()
            {
                if known_instance != instance || known_app != app || status != "active" {
                    return Err(invalid("application_conversation_restore_denied"));
                }
                if known_authorization != authorization {
                    if legacy_authorization != Some(known_authorization.as_str()) {
                        return Err(invalid("application_conversation_restore_denied"));
                    }
                    let mut statement = tx
                        .prepare(
                            "SELECT snapshot_json FROM ai_application_messages WHERE session_id=?1",
                        )
                        .map_err(database_error("read report authorization writers"))?;
                    let snapshots = statement
                        .query_map([id], |row| row.get::<_, String>(0))
                        .map_err(database_error("read report authorization writers"))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(database_error("read report authorization writers"))?;
                    for snapshot in snapshots {
                        let snapshot = decode(&snapshot)?;
                        if active(&snapshot)
                            || matches!(
                                snapshot
                                    .pointer("/writeRequest/approval/executionStatus")
                                    .and_then(Value::as_str),
                                Some("submitting" | "outcome_unknown")
                            )
                        {
                            return Err(invalid(
                                "application_report_authorization_reconciliation_required",
                            ));
                        }
                    }
                    drop(statement);
                    tx.execute("UPDATE ai_application_sessions SET authorization_digest=?2,revision=revision+1,updated_at=?3 WHERE session_id=?1", params![id,authorization,now])
                        .map_err(database_error("migrate report authorization"))?;
                }
            } else {
                tx.execute("INSERT INTO ai_application_sessions(session_id,instance_id,application_id,authorization_digest,status,created_at,updated_at) VALUES(?1,?2,?3,?4,'active',?5,?5)", params![id,instance,app,authorization,now]).map_err(database_error("create application conversation"))?;
            }
        } else {
            let (_, app, _, status, _) = existing
                .as_ref()
                .ok_or_else(|| invalid("application_conversation_missing"))?;
            if value
                .get("applicationId")
                .is_some_and(|v| v.as_str() != Some(app))
            {
                return Err(invalid("application_conversation_scope_denied"));
            }
            if matches!(action, "reserve" | "attach") && status != "active" {
                return Err(invalid("application_conversation_closed"));
            }
        }
        match action {
            "attach" => {
                let attachment = text(value, "attachmentId")?;
                let count: i64 = tx
                    .query_row(
                        "SELECT count(*) FROM ai_application_attachment_refs WHERE session_id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(database_error("read pending attachment count"))?;
                if count >= 8 {
                    return Err(invalid("ai_attachment_count_exceeded"));
                }
                if value["attachment"]["attachmentId"] != attachment
                    || !value["attachment"]["localPath"].is_string()
                {
                    return Err(invalid("application_attachment_reference_invalid"));
                }
                tx.execute("INSERT INTO ai_application_attachment_refs(session_id,attachment_id,metadata_json) VALUES(?1,?2,?3)",params![id,attachment,value["attachment"].to_string()]).map_err(database_error("bind application attachment"))?;
            }
            "discardAttachment" => {
                tx.execute("DELETE FROM ai_application_attachment_refs WHERE session_id=?1 AND attachment_id=?2",params![id,text(value,"attachmentId")?]).map_err(database_error("release application attachment"))?;
            }
            "reserve" => {
                let client = text(value, "clientMessageId")?;
                let hash = text(value, "contentDigest")?;
                let run = text(value, "runId")?;
                let known: Option<(String,String)> = tx.query_row("SELECT content_digest,run_id FROM ai_application_messages WHERE session_id=?1 AND client_message_id=?2",params![id,client],|r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error("read application message receipt"))?;
                if let Some((known_hash, known_run)) = known {
                    if known_hash != hash {
                        return Err(invalid("application_message_identity_conflict"));
                    }
                    tx.commit()
                        .map_err(database_error("read application message replay"))?;
                    return Ok(json!({"replayed":true,"runId":known_run}));
                }
                let mut statement = tx
                    .prepare(
                        "SELECT snapshot_json FROM ai_application_messages WHERE session_id=?1",
                    )
                    .map_err(database_error("read application writers"))?;
                let rows = statement
                    .query_map([id], |r| r.get::<_, String>(0))
                    .map_err(database_error("read application writers"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(database_error("read application writers"))?;
                if rows
                    .iter()
                    .map(|r| decode(r))
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .any(|s| {
                        active(s)
                            || matches!(
                                s.pointer("/writeRequest/approval/executionStatus")
                                    .and_then(Value::as_str),
                                Some("submitting" | "outcome_unknown")
                            )
                    })
                {
                    return Err(invalid("application_model_run_active"));
                }
                drop(statement);
                let snapshot = metadata(&value["snapshot"])?;
                if snapshot["runId"] != run
                    || snapshot["applicationId"]
                        != existing
                            .as_ref()
                            .ok_or_else(|| invalid("application_conversation_missing"))?
                            .1
                {
                    return Err(invalid("application_conversation_scope_denied"));
                }
                if value["context"].get("prompt").is_some() {
                    return Err(invalid("application_context_contains_message"));
                }
                tx.execute("INSERT INTO ai_application_messages(session_id,client_message_id,content_digest,run_id,snapshot_json,context_json,attachments_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![id,client,hash,run,snapshot.to_string(),value["context"].to_string(),value["attachments"].to_string(),now]).map_err(database_error("reserve application message"))?;
            }
            "update" => {
                let run = text(value, "runId")?;
                let old: String = tx.query_row("SELECT snapshot_json FROM ai_application_messages WHERE session_id=?1 AND run_id=?2",params![id,run],|r|r.get(0)).optional().map_err(database_error("read application run"))?.ok_or_else(||invalid("application_model_run_not_found"))?;
                let old = decode(&old)?;
                let next = metadata(&value["snapshot"])?;
                if old["runId"] != next["runId"]
                    || old["applicationId"] != next["applicationId"]
                    || old["modelId"] != next["modelId"]
                    || old["modelExecution"] != next["modelExecution"]
                    || (!old["replyUsage"].is_null() && old["replyUsage"] != next["replyUsage"])
                    || (!active(&old) && old["status"] != next["status"])
                {
                    return Err(invalid("application_run_revision_conflict"));
                }
                tx.execute("UPDATE ai_application_messages SET snapshot_json=?3,revision=revision+1,updated_at=?4 WHERE session_id=?1 AND run_id=?2",params![id,run,next.to_string(),now]).map_err(database_error("update application run"))?;
            }
            "close" => {
                tx.execute("UPDATE ai_application_sessions SET status='closed',revision=revision+1,updated_at=?2 WHERE session_id=?1 AND status='active'",params![id,now]).map_err(database_error("close application conversation"))?;
            }
            "beginCleanup" => {
                if existing
                    .as_ref()
                    .is_none_or(|s| !matches!(s.3.as_str(), "closed" | "cleaning"))
                {
                    return Err(invalid("application_cleanup_not_closed"));
                }
                let mut statement = tx
                    .prepare(
                        "SELECT snapshot_json FROM ai_application_messages WHERE session_id=?1",
                    )
                    .map_err(database_error("read application cleanup"))?;
                let rows = statement
                    .query_map([id], |r| r.get::<_, String>(0))
                    .map_err(database_error("read application cleanup"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(database_error("read application cleanup"))?;
                for raw in rows {
                    let snapshot = decode(&raw)?;
                    if active(&snapshot)
                        || matches!(
                            snapshot
                                .pointer("/writeRequest/approval/executionStatus")
                                .and_then(Value::as_str),
                            Some("submitting" | "outcome_unknown")
                        )
                    {
                        return Err(invalid("application_cleanup_reconciliation_required"));
                    }
                }
                drop(statement);
                tx.execute("UPDATE ai_application_sessions SET status='cleaning',revision=revision+1,updated_at=?2 WHERE session_id=?1",params![id,now]).map_err(database_error("begin application cleanup"))?;
            }
            "finishCleanup" => {
                if existing.as_ref().is_none_or(|s| s.3 != "cleaning") {
                    return Err(invalid("application_cleanup_not_started"));
                }
                let mut statement=tx.prepare("SELECT conversation_id FROM ai_application_thread_runs WHERE run_id IN (SELECT run_id FROM ai_application_messages WHERE session_id=?1)").map_err(database_error("read application cleanup journals"))?;
                let mut conversations = statement
                    .query_map([id], |r| r.get::<_, String>(0))
                    .map_err(database_error("read application cleanup journals"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(database_error("read application cleanup journals"))?;
                drop(statement);
                conversations.push(format!("application:{id}"));
                conversations.sort();
                conversations.dedup();
                for conversation in conversations {
                    for table in [
                        "ai_thread_message_links",
                        "ai_thread_generations",
                        "ai_thread_operations",
                        "ai_conversation_thread_bindings",
                    ] {
                        tx.execute(
                            &format!("DELETE FROM {table} WHERE conversation_id=?1"),
                            [&conversation],
                        )
                        .map_err(database_error("clean application thread journal"))?;
                    }
                }
                tx.execute("UPDATE ai_application_messages SET context_json='{}',attachments_json='[]' WHERE session_id=?1",[id]).map_err(database_error("clean application references"))?;
                tx.execute(
                    "DELETE FROM ai_application_attachment_refs WHERE session_id=?1",
                    [id],
                )
                .map_err(database_error(
                    "clean pending application attachment references",
                ))?;
                tx.execute("UPDATE ai_application_sessions SET status='cleaned',revision=revision+1,updated_at=?2 WHERE session_id=?1",params![id,now]).map_err(database_error("finish application cleanup"))?;
            }
            "read" | "open" | "openReport" | "openPage" => {}
            _ => return Err(invalid("application_conversation_action_invalid")),
        }
        let session=tx.query_row("SELECT application_id,status,revision FROM ai_application_sessions WHERE session_id=?1",[id],|r|Ok(json!({"sessionId":id,"applicationId":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"revision":r.get::<_,i64>(2)?}))).map_err(database_error("read application session"))?;
        let mut statement=tx.prepare("SELECT client_message_id,content_digest,run_id,snapshot_json,context_json,attachments_json,revision FROM ai_application_messages WHERE session_id=?1 ORDER BY created_at,rowid").map_err(database_error("read application messages"))?;
        let rows = statement
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(database_error("read application messages"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read application messages"))?;
        let messages=rows.into_iter().map(|(client,hash,run,snapshot,context,attachments,revision)|Ok(json!({"clientMessageId":client,"contentDigest":hash,"runId":run,"snapshot":decode(&snapshot)?,"context":decode(&context)?,"attachments":decode(&attachments)?,"revision":revision}))).collect::<Result<Vec<_>,PersistenceError>>()?;
        drop(statement);
        tx.commit()
            .map_err(database_error("commit application conversation"))?;
        let mut statement=self.connection.prepare("SELECT metadata_json FROM ai_application_attachment_refs WHERE session_id=?1 ORDER BY attachment_id").map_err(database_error("read pending application attachments"))?;
        let attachments = statement
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(database_error("read pending application attachments"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read pending application attachments"))?
            .into_iter()
            .map(|s| decode(&s))
            .collect::<Result<Vec<_>, _>>()?;
        let ids = attachments
            .iter()
            .map(|a| a["attachmentId"].clone())
            .collect::<Vec<_>>();
        Ok(
            json!({"session":session,"messages":messages,"pendingAttachmentIds":ids,"pendingAttachments":attachments,"replayed":false}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDatabase {
        path: std::path::PathBuf,
    }
    impl TestDatabase {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self {
                path: root.join("state.sqlite"),
            }
        }
    }
    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
        }
    }
    fn call(
        db: &mut SqlitePersistence,
        action: &str,
        payload: &Value,
    ) -> Result<Value, PersistenceError> {
        db.application_conversation(action, payload, 1)
    }
    #[test]
    fn cold_restart_preflight_preserves_unaccepted_input_and_blocks_running_or_unknown_writes() {
        let fixture = TestDatabase::new("application-restart-cold");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        for (session, status) in [("unaccepted", "queued"), ("active", "running")] {
            call(&mut db, "open", &json!({"sessionId":session,"instanceId":session,"applicationId":"app","authorizationDigest":"auth"})).unwrap();
            call(&mut db, "reserve", &json!({"sessionId":session,"clientMessageId":"input","contentDigest":"digest","runId":session,"snapshot":{"runId":session,"applicationId":"app","modelId":"model","status":status},"context":{},"attachments":[]})).unwrap();
        }
        let before = call(&mut db, "read", &json!({"sessionId":"active"})).unwrap();
        drop(db);
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert_eq!(
            call(&mut db, "restartBlockers", &json!({})).unwrap()["blockers"],
            json!(["application_model_run_active"])
        );
        assert_eq!(
            call(&mut db, "read", &json!({"sessionId":"active"})).unwrap(),
            before
        );
        call(&mut db, "update", &json!({"sessionId":"active","runId":"active","snapshot":{"runId":"active","applicationId":"app","modelId":"model","status":"failed","writeRequest":{"approval":{"executionStatus":"outcome_unknown"}}}})).unwrap();
        assert_eq!(
            call(&mut db, "restartBlockers", &json!({})).unwrap()["blockers"],
            json!(["application_write_outcome_unsettled"])
        );
        call(&mut db, "update", &json!({"sessionId":"active","runId":"active","snapshot":{"runId":"active","applicationId":"app","modelId":"model","status":"failed","writeRequest":null}})).unwrap();
        assert_eq!(
            call(&mut db, "restartBlockers", &json!({})).unwrap()["blockers"],
            json!([])
        );
        assert_eq!(
            call(&mut db, "read", &json!({"sessionId":"unaccepted"})).unwrap()["messages"][0]["snapshot"]
                ["status"],
            "queued"
        );
    }

    #[test]
    fn restart_preflight_keeps_dispatched_native_work_blocked_after_writer_release() {
        for dispatched in [false, true] {
            let fixture = TestDatabase::new("application-restart-native");
            let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            let conversation = "application:page";
            let run = "app_run_restart";
            call(&mut db, "open", &json!({"sessionId":"page","instanceId":"instance","applicationId":"app","authorizationDigest":"auth"})).unwrap();
            call(&mut db, "reserve", &json!({"sessionId":"page","clientMessageId":"input","contentDigest":"digest","runId":run,"snapshot":{"runId":run,"applicationId":"app","modelId":"model","status":"queued"},"context":{},"attachments":[]})).unwrap();
            assert_eq!(
                call(&mut db, "restartBlockers", &json!({})).unwrap()["blockers"],
                json!([])
            );
            let request = json!({"conversationId":conversation,"operationId":"operation","runId":run,"ownerInstanceId":"owner","kind":"create","input":{},"expectedBindingRevision":0,"protocolVersion":7,"applicationScope":{"applicationId":"app","sessionKind":"application_page"}});
            db.prepare_ai_thread_operation(&request, 2).unwrap();
            assert_eq!(
                call(&mut db, "restartBlockers", &json!({})).unwrap()["blockers"],
                json!(["application_model_run_active"])
            );
            if dispatched {
                db.acknowledge_ai_thread_operation(&json!({"conversationId":conversation,"operationId":"operation","runId":run,"ownerInstanceId":"owner","event":"dispatch","receipt":null}), 3).unwrap();
            }
            db.release_ai_thread_writer(&json!({"conversationId":conversation,"runId":run,"ownerInstanceId":"owner","processReaped":true}), 4).unwrap();
            let blockers = call(&mut db, "restartBlockers", &json!({})).unwrap();
            assert_eq!(
                blockers["blockers"],
                if dispatched {
                    json!(["application_model_run_active"])
                } else {
                    json!([])
                }
            );
            let history = call(&mut db, "read", &json!({"sessionId":"page"})).unwrap();
            assert_eq!(history["messages"].as_array().unwrap().len(), 1);
            assert_eq!(history["messages"][0]["snapshot"]["status"], "queued");
        }
    }
    #[test]
    fn page_upgrade_preserves_matching_history_and_isolates_changed_authorizations() {
        let fixture = TestDatabase::new("page-authorization");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let instance = "profile-scoped-instance";
        let legacy = format!("model_session_{:x}", Sha256::digest(instance.as_bytes()));
        let original = json!({"sessionId":legacy,"instanceId":instance,"applicationId":"policy","authorizationDigest":"legacy-v1"});
        call(&mut db, "open", &original).unwrap();
        call(&mut db,"reserve",&json!({"sessionId":legacy,"clientMessageId":"input","contentDigest":"digest","runId":"run","snapshot":{"runId":"run","applicationId":"policy","status":"completed","nativeThreadId":"original-thread","nativeTurnId":"original-turn"},"context":{},"attachments":[]})).unwrap();
        let page = |scope: &str, old: &str| {
            let auth = format!("page-v1:{:x}", Sha256::digest(scope.as_bytes()));
            json!({"sessionId":format!("model_page_{:x}",Sha256::digest(auth.as_bytes())),"legacySessionId":legacy,"instanceId":instance,"applicationId":"policy","authorizationDigest":auth,"legacyAuthorizationDigest":old})
        };
        let current = page("member-one-version-one", "legacy-v1");
        let adopted = call(&mut db, "openPage", &current).unwrap();
        assert_eq!(adopted["session"]["sessionId"], legacy);
        assert_eq!(
            adopted["messages"][0]["snapshot"]["nativeThreadId"],
            "original-thread"
        );
        assert_eq!(call(&mut db, "openPage", &current).unwrap(), adopted);
        let upgraded = page("member-one-version-two", "legacy-v2");
        let fresh = call(&mut db, "openPage", &upgraded).unwrap();
        assert_eq!(fresh["session"]["sessionId"], upgraded["sessionId"]);
        assert!(fresh["messages"].as_array().unwrap().is_empty());
        assert_eq!(call(&mut db, "openPage", &current).unwrap(), adopted);
        let another_member = call(
            &mut db,
            "openPage",
            &page("member-two-version-two", "legacy-v2"),
        )
        .unwrap();
        assert_ne!(
            another_member["session"]["sessionId"],
            fresh["session"]["sessionId"]
        );
        let mut denied = original;
        denied["authorizationDigest"] = upgraded["authorizationDigest"].clone();
        assert!(call(&mut db, "open", &denied).is_err());
    }

    #[test]
    fn cold_instance_discovery_lists_real_ordinary_scopes_and_preserves_reports_and_foreign_rows() {
        let fixture = TestDatabase::new("cold-instance");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        for (session, instance, application) in [
            ("one", "host-instance", "policy"),
            ("two", "host-instance:current-scope", "policy"),
            ("model_report_resource", "report-resource", "policy"),
            ("foreign", "other-instance", "policy"),
            ("other-app", "other-app-instance", "other"),
        ] {
            call(&mut db,"open",&json!({"sessionId":session,"instanceId":instance,"applicationId":application,"authorizationDigest":"approved"})).unwrap();
        }
        let report_before = call(
            &mut db,
            "read",
            &json!({"sessionId":"model_report_resource"}),
        )
        .unwrap();
        let foreign_before = call(&mut db, "read", &json!({"sessionId":"foreign"})).unwrap();
        assert_eq!(
            call(
                &mut db,
                "listInstance",
                &json!({"instanceId":"host-instance","applicationId":"policy"})
            )
            .unwrap(),
            json!({"sessionIds":["one","two"]})
        );
        assert_eq!(
            call(
                &mut db,
                "read",
                &json!({"sessionId":"model_report_resource"})
            )
            .unwrap(),
            report_before
        );
        assert_eq!(
            call(&mut db, "read", &json!({"sessionId":"foreign"})).unwrap(),
            foreign_before
        );
    }

    #[test]
    fn page_upgrade_cannot_bypass_an_unsettled_old_writer() {
        for mut snapshot in [
            json!({"status":"running"}),
            json!({"status":"failed","writeRequest":{"approval":{"executionStatus":"outcome_unknown"}}}),
        ] {
            snapshot["runId"] = json!("run");
            snapshot["applicationId"] = json!("app");
            let fixture = TestDatabase::new("page-unsettled");
            let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            let instance = "instance";
            let legacy = format!("model_session_{:x}", Sha256::digest(instance.as_bytes()));
            call(&mut db,"open",&json!({"sessionId":legacy,"instanceId":instance,"applicationId":"app","authorizationDigest":"old"})).unwrap();
            call(&mut db,"reserve",&json!({"sessionId":legacy,"clientMessageId":"input","contentDigest":"digest","runId":"run","snapshot":snapshot,"context":{},"attachments":[]})).unwrap();
            let auth = format!("page-v1:{:x}", Sha256::digest(b"current"));
            let request = json!({"sessionId":format!("model_page_{:x}",Sha256::digest(auth.as_bytes())),"legacySessionId":legacy,"instanceId":instance,"applicationId":"app","authorizationDigest":auth,"legacyAuthorizationDigest":"new"});
            assert!(call(&mut db, "openPage", &request).is_err());
            let original = call(&mut db, "read", &json!({"sessionId":legacy})).unwrap();
            assert_eq!(original["messages"][0]["snapshot"], snapshot);
        }
    }

    #[test]
    fn stable_message_receipts_survive_reopen_and_conflicts_cannot_dispatch() {
        let fixture = TestDatabase::new("application-conversation");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let open =
            json!({"sessionId":"s","instanceId":"i","applicationId":"a","authorizationDigest":"d"});
        call(&mut db, "open", &open).unwrap();
        let reserve = json!({"sessionId":"s","clientMessageId":"m","contentDigest":"h","runId":"r","snapshot":{"runId":"r","applicationId":"a","modelId":"model","status":"queued","output":"must not persist"},"context":{},"attachments":[]});
        call(&mut db, "reserve", &reserve).unwrap();
        drop(db);
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert_eq!(
            call(&mut db, "reserve", &reserve).unwrap()["replayed"],
            true
        );
        let mut conflict = reserve.clone();
        conflict["contentDigest"] = json!("other");
        assert!(call(&mut db, "reserve", &conflict).is_err());
        let mut concurrent = reserve;
        concurrent["clientMessageId"] = json!("m2");
        concurrent["runId"] = json!("r2");
        assert!(call(&mut db, "reserve", &concurrent).is_err());
        let read = call(&mut db, "read", &json!({"sessionId":"s"})).unwrap();
        assert!(read["messages"][0]["snapshot"].get("output").is_none());
        let mut wrong = open.clone();
        wrong["authorizationDigest"] = json!("changed");
        assert!(call(&mut db, "open", &wrong).is_err());
        call(&mut db, "close", &json!({"sessionId":"s"})).unwrap();
        assert!(call(&mut db, "beginCleanup", &json!({"sessionId":"s"})).is_err());
        assert!(call(&mut db, "open", &open).is_err());
    }

    #[test]
    fn report_authorization_migration_preserves_messages_attachments_and_reopen() {
        let fixture = TestDatabase::new("report-authorization");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let id = format!("model_report_{}", "a".repeat(64));
        let app = "com.cilai.environmental.policy";
        let legacy = "b".repeat(64);
        let current = format!("report-v1:{}", "c".repeat(64));
        let old_open = json!({"sessionId":id,"instanceId":id,"applicationId":app,"authorizationDigest":legacy});
        call(&mut db, "open", &old_open).unwrap();
        call(&mut db, "reserve", &json!({"sessionId":id,"clientMessageId":"original","contentDigest":"original-digest","runId":"r","snapshot":{"runId":"r","applicationId":app,"modelId":"model","status":"completed"},"context":{"report":"original"},"attachments":[]})).unwrap();
        call(&mut db, "attach", &json!({"sessionId":id,"attachmentId":"original-attachment","attachment":{"attachmentId":"original-attachment","localPath":"/synthetic/material"}})).unwrap();
        let mut report_open = json!({"sessionId":id,"instanceId":id,"applicationId":app,"authorizationDigest":current,"legacyAuthorizationDigest":legacy});
        let migrated = call(&mut db, "openReport", &report_open).unwrap();
        assert_eq!(migrated["messages"][0]["clientMessageId"], "original");
        assert_eq!(migrated["messages"][0]["contentDigest"], "original-digest");
        assert_eq!(
            migrated["pendingAttachmentIds"],
            json!(["original-attachment"])
        );
        assert!(call(&mut db, "open", &old_open).is_err());
        drop(db);
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        report_open["legacyAuthorizationDigest"] = json!("d".repeat(64));
        let reopened = call(&mut db, "openReport", &report_open).unwrap();
        assert_eq!(reopened["messages"], migrated["messages"]);
        assert_eq!(
            reopened["pendingAttachmentIds"],
            migrated["pendingAttachmentIds"]
        );
        assert_eq!(
            reopened["session"]["revision"],
            migrated["session"]["revision"]
        );
    }

    #[test]
    fn report_authorization_rejects_unverified_legacy_scope_and_closed_sessions() {
        let fixture = TestDatabase::new("report-authorization-deny");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let id = format!("model_report_{}", "a".repeat(64));
        let app = "com.cilai.environmental.policy";
        let legacy = "b".repeat(64);
        let old_open = json!({"sessionId":id,"instanceId":id,"applicationId":app,"authorizationDigest":legacy});
        call(&mut db, "open", &old_open).unwrap();
        let report_open = json!({"sessionId":id,"instanceId":id,"applicationId":app,"authorizationDigest":format!("report-v1:{}", "c".repeat(64)),"legacyAuthorizationDigest":legacy});
        for (field, invalid_value) in [
            ("legacyAuthorizationDigest", json!("d".repeat(64))),
            ("instanceId", json!("other-instance")),
            ("applicationId", json!("other-application")),
            ("authorizationDigest", json!("c".repeat(64))),
        ] {
            let mut wrong = report_open.clone();
            wrong[field] = invalid_value;
            assert!(call(&mut db, "openReport", &wrong).is_err());
            assert!(call(&mut db, "open", &old_open).is_ok());
        }
        call(&mut db, "openReport", &report_open).unwrap();
        let mut changed_scope = report_open.clone();
        changed_scope["authorizationDigest"] = json!(format!("report-v1:{}", "d".repeat(64)));
        assert!(call(&mut db, "openReport", &changed_scope).is_err());
        call(&mut db, "close", &json!({"sessionId":id})).unwrap();
        assert!(call(&mut db, "openReport", &report_open).is_err());
    }

    #[test]
    fn report_authorization_migration_waits_for_original_execution_reconciliation() {
        let fixture = TestDatabase::new("report-authorization-active");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let id = format!("model_report_{}", "a".repeat(64));
        let app = "com.cilai.environmental.policy";
        let legacy = "b".repeat(64);
        let old_open = json!({"sessionId":id,"instanceId":id,"applicationId":app,"authorizationDigest":legacy});
        call(&mut db, "open", &old_open).unwrap();
        let report_open = json!({"sessionId":id,"instanceId":id,"applicationId":app,"authorizationDigest":format!("report-v1:{}", "c".repeat(64)),"legacyAuthorizationDigest":legacy});
        let mut snapshot =
            json!({"runId":"r","applicationId":app,"modelId":"model","status":"queued"});
        call(&mut db, "reserve", &json!({"sessionId":id,"clientMessageId":"original","contentDigest":"original-digest","runId":"r","snapshot":snapshot,"context":{},"attachments":[]})).unwrap();
        assert!(call(&mut db, "openReport", &report_open).is_err());
        assert!(call(&mut db, "open", &old_open).is_ok());
        snapshot["status"] = json!("completed");
        snapshot["writeRequest"] = json!({"approval":{"executionStatus":"outcome_unknown"}});
        call(
            &mut db,
            "update",
            &json!({"sessionId":id,"runId":"r","snapshot":snapshot}),
        )
        .unwrap();
        assert!(call(&mut db, "openReport", &report_open).is_err());
        snapshot["writeRequest"] = Value::Null;
        call(
            &mut db,
            "update",
            &json!({"sessionId":id,"runId":"r","snapshot":snapshot}),
        )
        .unwrap();
        assert!(call(&mut db, "openReport", &report_open).is_ok());
    }
    #[test]
    fn usage_reads_only_durable_metadata_without_reopening_or_exposing_page_content() {
        let fixture = TestDatabase::new("page-usage");
        let mut db = SqlitePersistence::open(&fixture.path, "test").unwrap();
        call(&mut db, "open", &json!({"sessionId":"usage-session","instanceId":"usage-instance","applicationId":"personal.page","authorizationDigest":"scope"})).unwrap();
        let snapshot = json!({"runId":"usage-run","applicationId":"personal.page","modelId":"recorded-model","status":"completed","startedAt":1,"completedAt":2,"eventPosition":3,
            "modelExecution":{"profileId":"source-profile","connectionId":"saved-connection","configRevision":2,"authGeneration":3,"modelId":"recorded-model","unknownPrivateField":"PRIVATE_IDENTITY"},
            "replyUsage":{"inputTokens":7,"cachedInputTokens":0,"outputTokens":2,"unknownPrivateField":"PRIVATE_USAGE"},
            "writeRequest":{"arguments":"PRIVATE_ARGUMENTS"},"modelTask":{"private":"PRIVATE_TASK"},"output":"PRIVATE_OUTPUT","partialOutput":"PRIVATE_PARTIAL"});
        call(&mut db, "reserve", &json!({"sessionId":"usage-session","clientMessageId":"usage-message","contentDigest":"digest","runId":"usage-run","snapshot":snapshot,"context":{"private":"PRIVATE_CONTEXT"},"attachments":[]})).unwrap();
        call(&mut db, "close", &json!({"sessionId":"usage-session"})).unwrap();
        drop(db);
        let mut db = SqlitePersistence::open(&fixture.path, "test").unwrap();
        let before: i64 = db
            .connection
            .query_row(
                "SELECT revision FROM ai_application_sessions WHERE session_id='usage-session'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let usage = call(&mut db, "usage", &json!({})).unwrap();
        assert_eq!(
            usage["snapshots"][0]["modelExecution"]["connectionId"],
            "saved-connection"
        );
        assert_eq!(usage["snapshots"][0]["replyUsage"]["inputTokens"], 7);
        assert_eq!(usage["snapshots"][0]["replyUsage"]["cachedInputTokens"], 0);
        assert!(!usage.to_string().contains("PRIVATE_"));
        assert_eq!(db.connection.query_row("SELECT status,revision FROM ai_application_sessions WHERE session_id='usage-session'", [], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?))).unwrap(), ("closed".into(),before));
        assert!(call(&mut db, "usage", &json!({"sessionId":"unexpected"})).is_err());
    }
    #[test]
    fn recorded_page_execution_identity_and_observed_usage_cannot_be_reassigned() {
        let fixture = TestDatabase::new("page-usage-identity");
        let mut db = SqlitePersistence::open(&fixture.path, "test").unwrap();
        call(&mut db, "open", &json!({"sessionId":"s","instanceId":"i","applicationId":"a","authorizationDigest":"scope"})).unwrap();
        let mut snapshot = json!({"runId":"r","applicationId":"a","modelId":"m","status":"running","modelExecution":{"connectionId":"original"}});
        call(&mut db, "reserve", &json!({"sessionId":"s","clientMessageId":"c","contentDigest":"h","runId":"r","snapshot":snapshot,"context":{},"attachments":[]})).unwrap();
        snapshot["modelExecution"]["connectionId"] = json!("later-selection");
        assert!(
            call(
                &mut db,
                "update",
                &json!({"sessionId":"s","runId":"r","snapshot":snapshot})
            )
            .is_err()
        );
        snapshot["modelExecution"]["connectionId"] = json!("original");
        snapshot["status"] = json!("completed");
        snapshot["replyUsage"] = json!({"inputTokens":1,"cachedInputTokens":0,"outputTokens":1});
        call(
            &mut db,
            "update",
            &json!({"sessionId":"s","runId":"r","snapshot":snapshot}),
        )
        .unwrap();
        snapshot["replyUsage"] = Value::Null;
        assert!(
            call(
                &mut db,
                "update",
                &json!({"sessionId":"s","runId":"r","snapshot":snapshot})
            )
            .is_err()
        );
    }
}
