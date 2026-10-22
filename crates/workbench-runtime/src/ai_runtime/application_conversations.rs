use super::{
    AiRuntimeManager, AppServerSupervisor, ApplicationAiContextSnapshot, ApplicationModelRunRecord,
    ApplicationModelRunSnapshot, HashSet, MessageAttachment, PathBuf, Value,
    bundled_codex_harness_executable, data_dir_for_state, fs, lock, persist_locked_or_restore,
    persistent_threads, remove_file_if_present, safe_storage_id,
};
use serde_json::json;

impl AiRuntimeManager {
    pub(super) fn application_store(
        server: &AppServerSupervisor,
        action: &str,
        payload: &Value,
    ) -> Result<Value, String> {
        server.request_result(
            "aiApplication/execute",
            &json!({"action":action,"payload":payload}),
        )
    }
    /// # Errors
    /// Rejects restoration into a different application or authorization generation.
    pub fn open_application_conversation(
        &self,
        session: &str,
        instance: &str,
        application: &str,
        authorization: &str,
    ) -> Result<Value, String> {
        let server = lock(&self.inner)
            .server
            .clone()
            .ok_or("ai_runtime_not_initialized")?;
        Self::application_store(
            &server,
            "open",
            &json!({"sessionId":session,"instanceId":instance,"applicationId":application,"authorizationDigest":authorization}),
        )
    }
    /// # Errors
    /// 当前报告授权或存量授权不能精确核验时拒绝恢复。
    pub fn open_report_application_conversation(
        &self,
        session: &str,
        application: &str,
        authorization: &str,
        legacy_authorization: &str,
    ) -> Result<Value, String> {
        let server = lock(&self.inner)
            .server
            .clone()
            .ok_or("ai_runtime_not_initialized")?;
        Self::application_store(
            &server,
            "openReport",
            &json!({"sessionId":session,"instanceId":session,"applicationId":application,"authorizationDigest":authorization,"legacyAuthorizationDigest":legacy_authorization}),
        )
    }
    pub(crate) fn persist_application_run(&self, run: &str) -> Result<(), String> {
        let (server, session, snapshot, pid) = {
            let state = lock(&self.inner);
            let record = state
                .application_model_runs
                .get(run)
                .ok_or("application_model_run_not_found")?;
            let Some(session) = record.persistent_session_id.clone() else {
                return Ok(());
            };
            (
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                session,
                record.snapshot.clone(),
                record.pid,
            )
        };
        let mut snapshot =
            serde_json::to_value(snapshot).map_err(|_| "application_model_run_invalid")?;
        snapshot["pid"] = json!(pid);
        snapshot["ownerInstanceId"] = json!(persistent_threads::owner_instance_id());
        Self::application_store(
            &server,
            "update",
            &json!({"sessionId":session,"runId":run,"snapshot":snapshot}),
        )?;
        Ok(())
    }
    /// # Errors
    /// 应用持久执行及未决写入状态无法核实时拒绝重启。
    pub fn application_restart_blockers(&self) -> Result<Vec<String>, String> {
        let server = lock(&self.inner)
            .server
            .clone()
            .ok_or("ai_runtime_not_initialized")?;
        let result = Self::application_store(&server, "restartBlockers", &json!({}))?;
        serde_json::from_value(result["blockers"].clone())
            .map_err(|_| "application_restart_preflight_invalid".to_owned())
    }
    /// # Errors
    /// Attachment ownership must be durably recorded before returning it to the page.
    pub fn application_attachment_reference(
        &self,
        session: &str,
        attachment: &str,
        attach: bool,
    ) -> Result<(), String> {
        let (server, metadata) = {
            let state = lock(&self.inner);
            (
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                if attach {
                    serde_json::to_value(
                        &state
                            .pending_attachments
                            .get(attachment)
                            .ok_or("ai_attachment_not_found")?
                            .attachment,
                    )
                    .map_err(|_| "ai_attachment_invalid")?
                } else {
                    Value::Null
                },
            )
        };
        Self::application_store(
            &server,
            if attach {
                "attach"
            } else {
                "discardAttachment"
            },
            &json!({"sessionId":session,"attachmentId":attachment,"attachment":metadata}),
        )?;
        Ok(())
    }
    /// # Errors
    /// Only an attachment owned by this application session can be discarded.
    pub fn discard_application_attachment(
        &self,
        session: &str,
        attachment: &str,
    ) -> Result<(), String> {
        let (server, root, cached) = {
            let state = lock(&self.inner);
            (
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                data_dir_for_state(&state)?.join("attachments"),
                state.pending_attachments.contains_key(attachment),
            )
        };
        let stored = Self::application_store(&server, "read", &json!({"sessionId":session}))?;
        let metadata = stored["pendingAttachments"]
            .as_array()
            .and_then(|a| a.iter().find(|a| a["attachmentId"] == attachment))
            .ok_or("application_attachment_not_owned")?;
        if cached {
            self.discard_attachment(attachment)?;
        } else {
            let path = PathBuf::from(
                metadata["localPath"]
                    .as_str()
                    .ok_or("application_attachment_reference_invalid")?,
            );
            if path.parent() != Some(root.as_path()) {
                return Err("application_attachment_reference_invalid".into());
            }
            remove_file_if_present(&path).map_err(|_| "application_attachment_cleanup_failed")?;
        }
        self.application_attachment_reference(session, attachment, false)
    }
    fn consume_application_attachments(&self, session: &str, entry: &Value) -> Result<(), String> {
        let attachments: Vec<MessageAttachment> =
            serde_json::from_value(entry["attachments"].clone())
                .map_err(|_| "application_input_attachments_invalid")?;
        let mut state = lock(&self.inner);
        let previous = state.persisted.clone();
        let mut changed = false;
        for attachment in &attachments {
            changed |= state
                .persisted
                .draft_attachments
                .remove(&attachment.attachment_id)
                .is_some();
        }
        if changed {
            persist_locked_or_restore(&mut state, previous)?;
        }
        for attachment in &attachments {
            state.pending_attachments.remove(&attachment.attachment_id);
        }
        drop(state);
        for attachment in attachments {
            self.application_attachment_reference(session, &attachment.attachment_id, false)?;
        }
        Ok(())
    }
    /// # Errors
    /// Revokes the session permanently; unresolved operations retain their recovery material.
    pub fn close_application_conversation(&self, session: &str) -> Result<(), String> {
        let server = lock(&self.inner)
            .server
            .clone()
            .ok_or("ai_runtime_not_initialized")?;
        match Self::application_store(&server, "close", &json!({"sessionId":session})) {
            Ok(_) => {
                self.cleanup_application_conversation(session)?;
                Ok(())
            }
            Err(e) if e.contains("application_conversation_missing") => Ok(()),
            Err(e) => Err(e),
        }
    }
    #[allow(clippy::too_many_lines)]
    pub(super) fn cleanup_application_conversation(&self, session: &str) -> Result<bool, String> {
        let _operation = self.begin_capability_operation()?;
        let (server, data_dir, pending) = {
            let state = lock(&self.inner);
            let pending = state
                .persisted
                .conversation_session
                .get("applicationHandoffs")
                .and_then(Value::as_array)
                .is_some_and(|q| q.iter().any(|h| h["sourceSessionId"] == session));
            (
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                data_dir_for_state(&state)?,
                pending,
            )
        };
        let stored = Self::application_store(&server, "read", &json!({"sessionId":session}))?;
        if pending
            || !matches!(
                stored["session"]["status"].as_str(),
                Some("closed" | "cleaning")
            )
        {
            return Ok(false);
        }
        if stored["session"]["status"] == "closed"
            && stored["messages"].as_array().is_some_and(|entries| {
                entries.iter().any(|entry| {
                    matches!(
                        entry["snapshot"]["status"].as_str(),
                        Some(
                            "queued"
                                | "running"
                                | "waiting_confirmation"
                                | "submitting"
                                | "terminating"
                                | "recovery_required"
                        )
                    )
                })
            })
        {
            self.read_application_conversation(
                session,
                stored["session"]["applicationId"]
                    .as_str()
                    .ok_or("application_cleanup_response_invalid")?,
            )?;
        }
        let stored = Self::application_store(&server, "read", &json!({"sessionId":session}))?;
        let mut conversations = vec![format!("application:{session}")];
        for entry in stored["messages"]
            .as_array()
            .ok_or("application_cleanup_response_invalid")?
        {
            if entry["snapshot"]
                .get("modelTask")
                .is_some_and(|t| !t.is_null())
            {
                conversations.push(format!(
                    "application-task:{}",
                    entry["runId"]
                        .as_str()
                        .ok_or("application_cleanup_response_invalid")?
                ));
            }
        }
        let mut native_threads = HashSet::new();
        for conversation in conversations {
            let journal =
                server.request_result("aiThread/get", &json!({"conversationId":conversation}))?;
            if journal["binding"]["activeRunId"].is_string()
                || journal["operations"].as_array().is_some_and(|ops| {
                    ops.iter().any(|o| {
                        matches!(
                            o["state"].as_str(),
                            Some("prepared" | "dispatched" | "uncertain")
                        )
                    })
                })
            {
                return Ok(false);
            }
            if let Some(thread) = journal["binding"]["nativeThreadId"].as_str() {
                native_threads.insert((thread.to_owned(), journal.to_string()));
            }
        }
        match Self::application_store(&server, "beginCleanup", &json!({"sessionId":session})) {
            Ok(_) => {}
            Err(e) if e.contains("application_cleanup_reconciliation_required") => {
                return Ok(false);
            }
            Err(e) => return Err(e),
        }
        if !native_threads.is_empty() {
            for (thread, journal) in native_threads {
                let journal: Value = serde_json::from_str(&journal)
                    .map_err(|_| "application_thread_scope_invalid")?;
                self.with_application_native_scope(&journal, |service| {
                    if service.catalog()?.contains_key(&thread) {
                        service.request("thread/delete", &json!({"threadId":thread}))?;
                    }
                    Ok(())
                })?;
            }
        }
        for id in stored["pendingAttachmentIds"]
            .as_array()
            .ok_or("application_cleanup_attachments_invalid")?
        {
            self.discard_application_attachment(
                session,
                id.as_str()
                    .ok_or("application_cleanup_attachments_invalid")?,
            )?;
        }
        let root = data_dir.join("attachments");
        let entries = stored["messages"]
            .as_array()
            .ok_or("application_conversation_storage_invalid")?;
        for entry in entries {
            let attachments: Vec<MessageAttachment> =
                serde_json::from_value(entry["attachments"].clone())
                    .map_err(|_| "application_cleanup_attachments_invalid")?;
            for attachment in attachments {
                let path = PathBuf::from(attachment.local_path);
                if path.parent() != Some(root.as_path()) {
                    return Err("application_cleanup_attachment_path_invalid".into());
                }
                remove_file_if_present(&path)
                    .map_err(|e| format!("application_cleanup_attachment_failed:{e}"))?;
            }
        }
        for entry in entries {
            let run = entry["runId"]
                .as_str()
                .filter(|id| safe_storage_id(id))
                .ok_or("application_cleanup_run_invalid")?;
            for suffix in [
                ".response.md",
                ".stdout.log",
                ".stderr.log",
                ".harness.jsonl",
                ".harness.model-catalog.json",
                ".harness.tool-labels.json",
                ".thread-check.json",
            ] {
                remove_file_if_present(&data_dir.join("runs").join(format!("{run}{suffix}")))
                    .map_err(|e| format!("application_cleanup_log_failed:{e}"))?;
            }
            for suffix in ["-frames", ".skills"] {
                let path = data_dir.join("runs").join(format!("{run}{suffix}"));
                if path.exists() {
                    fs::remove_dir_all(path)
                        .map_err(|e| format!("application_cleanup_cache_failed:{e}"))?;
                }
            }
        }
        Self::application_store(&server, "finishCleanup", &json!({"sessionId":session}))?;
        let mut state = lock(&self.inner);
        state
            .application_model_runs
            .retain(|_, r| r.persistent_session_id.as_deref() != Some(session));
        Ok(true)
    }
    pub(super) fn resume_application_cleanup(&self) -> Result<(), String> {
        let server = lock(&self.inner)
            .server
            .clone()
            .ok_or("ai_runtime_not_initialized")?;
        let pending = Self::application_store(&server, "listPending", &json!({}))?;
        for session in pending["sessions"]
            .as_array()
            .ok_or("application_cleanup_response_invalid")?
        {
            let id = session["sessionId"]
                .as_str()
                .ok_or("application_cleanup_response_invalid")?;
            self.cleanup_application_conversation(id)?;
        }
        Ok(())
    }
    pub(crate) fn application_message_receipt(
        &self,
        session: &str,
        client: &str,
        digest: &str,
    ) -> Result<Option<ApplicationModelRunSnapshot>, String> {
        let server = lock(&self.inner)
            .server
            .clone()
            .ok_or("ai_runtime_not_initialized")?;
        let stored = Self::application_store(&server, "read", &json!({"sessionId":session}))?;
        let Some(message) = stored["messages"]
            .as_array()
            .ok_or("application_conversation_storage_invalid")?
            .iter()
            .find(|m| m["clientMessageId"] == client)
        else {
            return Ok(None);
        };
        if message["contentDigest"] != digest {
            return Err("application_message_identity_conflict".into());
        }
        let run = message["runId"]
            .as_str()
            .ok_or("application_conversation_storage_invalid")?;
        if let Ok(snapshot) = self.application_model_run(run) {
            return Ok(Some(snapshot));
        }
        let application = stored["session"]["applicationId"]
            .as_str()
            .ok_or("application_conversation_storage_invalid")?;
        self.read_application_conversation(session, application)?;
        self.application_model_run(run).map(Some)
    }
    pub(crate) fn application_dispatch_digest(
        context: &ApplicationAiContextSnapshot,
        model: &str,
        instruction: &str,
        prompt: &str,
        attachments: &[String],
    ) -> String {
        persistent_threads::digest(
            &json!({"modelId":model,"content":prompt.trim(),"additionalContext":instruction.trim(),"attachments":attachments,"applicationId":context.application_id,"version":context.version,"pageRevision":context.page_revision,"contexts":context.contexts,"task":context.model_task,"localMaterialPaths":context.local_material_paths}),
        )
    }
    /// # Errors
    /// Fails if native history or its accepted-input mapping cannot be verified.
    #[allow(clippy::too_many_lines)]
    pub fn read_application_conversation(
        &self,
        session: &str,
        application: &str,
    ) -> Result<Value, String> {
        let _operation = self.begin_capability_operation()?;
        let server = {
            let state = lock(&self.inner);
            state.server.clone().ok_or("ai_runtime_not_initialized")?
        };
        let stored = Self::application_store(
            &server,
            "read",
            &json!({"sessionId":session,"applicationId":application}),
        )?;
        let conversation = format!("application:{session}");
        let journal =
            server.request_result("aiThread/get", &json!({"conversationId":conversation}))?;
        let (journal, turns, recovery_error) =
            self.with_application_native_scope(&journal, |service| {
                let recovered = persistent_threads::recover(
                    &server,
                    &conversation,
                    journal.clone(),
                    &bundled_codex_harness_executable()?,
                    service.home(),
                );
                let recovery_error = recovered.as_ref().err().cloned();
                let journal = recovered.unwrap_or_else(|_| journal.clone());
                let turns = journal["binding"]["nativeThreadId"]
                    .as_str()
                    .map_or_else(|| Ok(Vec::new()), |thread| service.turns(thread))?;
                Ok((journal, turns, recovery_error))
            })?;
        let mut messages = Vec::new();
        let mut blocked = false;
        for entry in stored["messages"]
            .as_array()
            .ok_or("application_conversation_storage_invalid")?
        {
            let run = entry["runId"]
                .as_str()
                .ok_or("application_conversation_storage_invalid")?;
            let client = entry["clientMessageId"]
                .as_str()
                .ok_or("application_conversation_storage_invalid")?;
            let task_journal;
            let task_turns;
            let (journal, turns) = if entry["snapshot"]
                .get("modelTask")
                .is_some_and(|t| !t.is_null())
            {
                let task_conversation = format!("application-task:{run}");
                let stored_task = server
                    .request_result("aiThread/get", &json!({"conversationId":task_conversation}))?;
                (task_journal, task_turns) =
                    self.with_application_native_scope(&stored_task, |service| {
                        let journal = persistent_threads::recover(
                            &server,
                            &task_conversation,
                            stored_task.clone(),
                            &bundled_codex_harness_executable()?,
                            service.home(),
                        )?;
                        let turns = journal["binding"]["nativeThreadId"]
                            .as_str()
                            .map_or_else(|| Ok(Vec::new()), |thread| service.turns(thread))?;
                        Ok((journal, turns))
                    })?;
                (&task_journal, &task_turns)
            } else {
                (&journal, &turns)
            };
            let native = journal["binding"]["nativeThreadId"].as_str();
            let link = journal["messages"]
                .as_array()
                .and_then(|links| links.iter().find(|m| m["clientMessageId"] == client));
            let turn = link.and_then(|link| turns.iter().find(|t| t["id"] == link["nativeTurnId"]));
            let user = turn.and_then(|t| t["items"].as_array()).and_then(|items| {
                items
                    .iter()
                    .find(|i| i["type"] == "userMessage" && i["clientId"] == client)
            });
            let prompt = user.map(crate::codex_threads::user_text);
            if link.is_some() {
                self.consume_application_attachments(session, entry)?;
            }
            if link.is_some() && prompt.is_none() {
                return Err("application_history_input_missing".into());
            }
            let existing = lock(&self.inner).application_model_runs.get(run).cloned();
            let mut snapshot = existing.as_ref().map(|r| r.snapshot.clone()).unwrap_or(
                serde_json::from_value(entry["snapshot"].clone())
                    .map_err(|_| "application_conversation_storage_invalid")?,
            );
            let terminal_native = recovery_error.is_none()
                && journal["binding"]["activeRunId"].is_null()
                && journal["binding"]["status"] == "ready"
                && turn.is_some_and(|turn| {
                    matches!(
                        turn["status"].as_str(),
                        Some("completed" | "failed" | "interrupted")
                    )
                });
            if existing.is_none()
                || (terminal_native
                    && matches!(
                        snapshot.status.as_str(),
                        "completed" | "failed" | "cancelled"
                    ))
            {
                snapshot.thread_id = native.map(str::to_owned);
                snapshot.output = if let Some(task) = snapshot
                    .model_task
                    .as_ref()
                    .filter(|task| task.result_schema.is_some())
                {
                    if let Some(turn) = turn.filter(|turn| {
                        snapshot.status == "completed" || turn["status"] == "completed"
                    }) {
                        Some(crate::application_result::from_native_turn(task, turn)?)
                    } else {
                        None
                    }
                } else {
                    turn.and_then(crate::codex_threads::turn_final_reply)
                };
                if terminal_native {
                    snapshot.status = match turn.and_then(|turn| turn["status"].as_str()) {
                        Some("completed") => "completed",
                        Some("interrupted") => "cancelled",
                        _ => "failed",
                    }
                    .into();
                    snapshot.completed_at = snapshot.completed_at.or_else(|| {
                        turn.and_then(|turn| turn["completedAt"].as_i64())
                            .map(|seconds| seconds.saturating_mul(1000))
                    });
                    snapshot.error_reason = if snapshot.status == "failed" {
                        Some(
                            turn.and_then(|turn| turn["error"]["message"].as_str())
                                .unwrap_or("原生执行失败，请核对后重试。")
                                .to_owned(),
                        )
                    } else {
                        None
                    };
                    if snapshot.status != "failed" {
                        snapshot.error_code = None;
                    }
                }
                if matches!(
                    snapshot.status.as_str(),
                    "queued"
                        | "running"
                        | "waiting_confirmation"
                        | "submitting"
                        | "terminating"
                        | "recovery_required"
                ) {
                    if recovery_error.is_none()
                        && link.is_none()
                        && journal["binding"]["activeRunId"].is_null()
                        && journal["operations"].as_array().is_some_and(|ops| {
                            ops.iter()
                                .filter(|o| o["request"]["runId"] == run)
                                .all(|o| o["state"] == "rejected")
                        })
                    {
                        snapshot.status = "failed".into();
                        snapshot.error_reason =
                            Some("原请求未受理，输入仍保留，请确认后发送。".into());
                    } else if recovery_error.is_none()
                        && journal["binding"]["activeRunId"].is_null()
                        && journal["binding"]["status"] == "ready"
                        && turn.is_some_and(|t| {
                            matches!(
                                t["status"].as_str(),
                                Some("completed" | "failed" | "interrupted")
                            )
                        })
                    {
                        snapshot.status = match turn.and_then(|t| t["status"].as_str()) {
                            Some("completed") => "completed",
                            Some("interrupted") => "cancelled",
                            _ => "failed",
                        }
                        .into();
                        snapshot.completed_at = turn
                            .and_then(|t| t["completedAt"].as_i64())
                            .map(|n| n.saturating_mul(1000));
                    } else {
                        snapshot.status = "recovery_required".into();
                        snapshot.error_reason = Some("原执行结果待核实，请勿重复发送。".into());
                        blocked = true;
                    }
                }
                if snapshot.status == "completed"
                    && let Some(metadata) = entry["context"].get("resultContract")
                {
                    let result = turn
                        .ok_or_else(|| "report_result_history_invalid".to_owned())
                        .and_then(|turn| {
                            crate::report_result::ReportResultContract::recover(
                                metadata,
                                client,
                                entry["contentDigest"].as_str().unwrap_or_default(),
                                turn,
                            )
                        });
                    match result {
                        Ok(output) => snapshot.output = Some(output),
                        Err(error) => {
                            snapshot.status = "failed".into();
                            snapshot.output = None;
                            snapshot.error_code = super::application_failure_code(&error);
                            snapshot.error_reason = Some(error);
                        }
                    }
                }
                if let Some(write) = snapshot
                    .write_request
                    .as_mut()
                    .filter(|_| existing.is_none())
                {
                    match write.approval.execution_status.as_deref() {
                        Some("awaiting_confirmation") => {
                            write.approval.execution_status = Some("cancelled".into());
                        }
                        Some("submitting") => {
                            write.approval.execution_status = Some("outcome_unknown".into());
                        }
                        _ => {}
                    }
                }
                let mut context = entry["context"].clone();
                context["prompt"] = json!(prompt);
                let events = if let Some(record) = existing.as_ref() {
                    record.events.clone()
                } else if let Some(turn) = turn {
                    super::native_history::events(turn, snapshot.started_at)?
                } else {
                    Vec::new()
                };
                lock(&self.inner).application_model_runs.insert(
                    run.into(),
                    ApplicationModelRunRecord {
                        persistent_session_id: Some(session.into()),
                        handoff_context: context,
                        snapshot: snapshot.clone(),
                        events,
                        pid: existing.as_ref().and_then(|record| record.pid),
                        cancel_requested: existing
                            .as_ref()
                            .is_none_or(|record| record.cancel_requested),
                    },
                );
                self.persist_application_run(run)?;
            } else if matches!(
                snapshot.status.as_str(),
                "queued"
                    | "running"
                    | "waiting_confirmation"
                    | "submitting"
                    | "terminating"
                    | "recovery_required"
            ) {
                blocked = true;
            }
            if snapshot.write_request.as_ref().is_some_and(|r| {
                matches!(
                    r.approval.execution_status.as_deref(),
                    Some("submitting" | "outcome_unknown")
                )
            }) {
                blocked = true;
            }
            let acceptance = if link.is_some() {
                "accepted"
            } else if matches!(snapshot.status.as_str(), "failed" | "cancelled")
                && journal["operations"].as_array().is_some_and(|ops| {
                    ops.iter()
                        .filter(|o| o["request"]["runId"] == run)
                        .all(|o| o["state"] == "rejected")
                })
            {
                "not_accepted"
            } else {
                "pending"
            };
            self.persist_application_run(run)?;
            let mut public =
                serde_json::to_value(&snapshot).map_err(|_| "application_model_run_invalid")?;
            public
                .as_object_mut()
                .ok_or("application_model_run_invalid")?
                .remove("threadId");
            messages.push(json!({"clientMessageId":client,"runId":run,"acceptance":acceptance,"prompt":prompt,"run":public,"revision":entry["revision"]}));
        }
        Ok(
            json!({"modelSessionId":session,"status":stored["session"]["status"],"recoveryRequired":blocked,"messages":messages,"revision":stored["session"]["revision"]}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation_kernel::{self, ThreadInput};

    struct ScopedHistory {
        service: std::sync::Arc<crate::codex_threads::CodexThreadService>,
        execution: std::sync::Arc<crate::model_connections::ExecutionScope>,
    }
    impl ScopedHistory {
        fn new(manager: &AiRuntimeManager, server: &AppServerSupervisor) -> Self {
            use contexthub_protocol::{
                ModelExecutionPrepareParams, ModelPurpose, ModelRuntimeKind,
            };
            let selection = super::super::model_test_fixture::seed_simulated_connection(
                server,
                "history-fixture",
            );
            let validation=server.request_result("modelValidation/prepare",&json!({"runtimeKind":"application","connectionId":selection.connection_id,"modelId":selection.model_id,"layer":"tools","expectedConfigRevision":1,"expectedAuthGeneration":1})).unwrap();
            server.request_result("modelValidation/report",&json!({"validationId":validation["validationId"],"outcome":"passed","reason":"isolated_fixture","runtimeVersion":null,"serviceVersion":"isolated_fixture"})).unwrap();
            super::super::model_test_fixture::bind_purpose(
                server,
                &selection,
                ModelPurpose::PageAi,
            );
            let execution = crate::model_connections::ExecutionScope::prepare(
                manager,
                ModelExecutionPrepareParams {
                    launch_protocol: 1,
                    owner: None,
                    purpose: ModelPurpose::PageAi,
                    binding_id: None,
                    selection: None,
                    conversation_id: Some("application:page".into()),
                },
            )
            .unwrap();
            let service = std::sync::Arc::new(
                crate::codex_threads::CodexThreadService::new(
                    crate::supervisor::codex_harness_test_binary(),
                    crate::model_connections::scoped_home(
                        &execution.data_dir,
                        &execution.lease.snapshot.runtime_home_scope,
                        ModelRuntimeKind::Application,
                    ),
                )
                .with_profile_dir(execution.data_dir.clone())
                .with_history_server(server.clone())
                .with_execution_scope(std::sync::Arc::clone(&execution)),
            );
            Self { service, execution }
        }
        fn bind(&self, plan: &mut crate::persistent_threads::PersistentThread) {
            self.service
                .request("thread/loaded/list", &json!({}))
                .unwrap();
            let leases = self
                .execution
                .server
                .request_result("modelExecution/list", &json!({}))
                .unwrap();
            let registered = leases
                .as_array()
                .unwrap()
                .iter()
                .find(|lease| lease["leaseId"] == self.execution.lease.lease_id)
                .unwrap();
            plan.journal.writer_process = registered["processes"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .clone();
            plan.journal.runtime_scope = Some(crate::model_connections::runtime_scope(
                &self.execution.lease.snapshot,
                contexthub_protocol::ModelRuntimeKind::Application,
            ));
            plan.journal.model_execution_lease_id = Some(self.execution.lease.lease_id.clone());
            plan.journal.owner_instance_id = self.execution.lease.owner.instance_id.clone();
        }
        fn stop(&self) {
            self.service.shutdown_checked().unwrap();
            self.execution.finish_release().unwrap();
        }
    }

    fn stop_test_manager(manager: &AiRuntimeManager) {
        manager.shutdown();
        if let Some(service) = lock(&manager.inner).application_threads.take() {
            service.shutdown();
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn twenty_native_turns_restore_by_instance_and_close_cleans_only_temporary_history() {
        let _guard = crate::supervisor::app_server_test_guard();
        let root = super::super::model_test_fixture::fixture_root("application-recovery");
        let data = super::super::model_test_fixture::profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&data, &server).unwrap();
        manager
            .open_application_conversation("page", "instance", "app", "authorization")
            .unwrap();
        let scoped_history = ScopedHistory::new(&manager, &server);
        let service = std::sync::Arc::clone(&scoped_history.service);
        let workspace = data.join("ai-runtime/application-ai/app");
        fs::create_dir_all(&workspace).unwrap();
        let created=service.request("thread/start",&json!({"cwd":workspace,"approvalPolicy":"never","sandbox":"read-only","ephemeral":false})).unwrap();
        let native = created["thread"]["id"].as_str().unwrap();
        let turns=(1..=20).map(|n|json!({"id":format!("turn{n}"),"startedAt":n,"completedAt":n,"error":null,"messages":[{"id":format!("m{n}"),"role":"user","text":format!("原文{n}")},{"id":format!("answer{n}"),"role":"assistant","text":format!("回复{n}")}]})).collect::<Vec<_>>();
        service
            .import_history(&json!({"threadId":native,"name":"应用临时对话","turns":turns}))
            .unwrap();
        let accepted = manager
            .create_attachment(super::super::AttachmentCreateRequest {
                file_name: "accepted.png".into(),
                mime_type: "image/png".into(),
                size_bytes: 1,
            })
            .unwrap();
        manager
            .application_attachment_reference("page", &accepted.attachment_id, true)
            .unwrap();
        let accepted_metadata = lock(&manager.inner).pending_attachments[&accepted.attachment_id]
            .attachment
            .clone();
        let accepted_path = accepted_metadata.local_path.clone();
        for n in 1..=20 {
            let run = format!("app_run_{n}");
            let client = format!("m{n}");
            let mut snapshot = json!({"runId":run,"applicationId":"app","modelId":"model","modelLabel":"模型","status":"queued","startedAt":n*1000,"completedAt":n*1000});
            AiRuntimeManager::application_store(&server,"reserve",&json!({"sessionId":"page","clientMessageId":client,"contentDigest":"digest","runId":run,"snapshot":snapshot,"context":{"applicationName":"应用","version":"1.1","pageRevision":n,"contexts":[]},"attachments":if n==1{vec![accepted_metadata.clone()]}else{vec![]}})).unwrap();
            let mut plan = conversation_kernel::thread_plan(
                server.clone(),
                ThreadInput {
                    application_scope: Some(contexthub_protocol::AiThreadApplicationScope {
                        application_id: "app".into(),
                        session_kind:
                            contexthub_protocol::AiApplicationSessionKind::ApplicationPage,
                    }),
                    conversation_id: "application:page".into(),
                    run_id: run.clone(),
                    message_id: client.clone(),
                    client_message_id: client.clone(),
                    content_digest: "digest".into(),
                },
                (n > 1).then(|| native.to_owned()),
                None,
            );
            scoped_history.bind(&mut plan);
            plan.journal
                .prepare(
                    "thread",
                    if n == 1 { "create" } else { "resume" },
                    json!({}),
                )
                .unwrap();
            plan.journal
                .record("thread", "dispatch", Value::Null)
                .unwrap();
            plan.journal
                .record("thread", "acknowledge", json!({"nativeThreadId":native}))
                .unwrap();
            plan.journal
                .prepare_input(&client, &client, "digest", false, &format!("原文{n}"))
                .unwrap();
            if n == 1 {
                assert_eq!(
                    AiRuntimeManager::application_store(
                        &server,
                        "read",
                        &json!({"sessionId":"page"})
                    )
                    .unwrap()["pendingAttachmentIds"][0],
                    accepted.attachment_id
                );
            }
            plan.journal
                .accept_input(&client, native, &format!("turn{n}"))
                .unwrap();
            service.shutdown_checked().unwrap();
            plan.journal.release().unwrap();
            snapshot["status"] = json!("completed");
            AiRuntimeManager::application_store(
                &server,
                "update",
                &json!({"sessionId":"page","runId":run,"snapshot":snapshot}),
            )
            .unwrap();
        }
        let pending = manager
            .create_attachment(super::super::AttachmentCreateRequest {
                file_name: "pending.png".into(),
                mime_type: "image/png".into(),
                size_bytes: 1,
            })
            .unwrap();
        manager
            .application_attachment_reference("page", &pending.attachment_id, true)
            .unwrap();
        let pending_path = lock(&manager.inner).pending_attachments[&pending.attachment_id]
            .attachment
            .local_path
            .clone();
        let restored = AiRuntimeManager::new();
        restored.initialize(&data, &server).unwrap();
        restored
            .open_application_conversation("page", "instance", "app", "authorization")
            .unwrap();
        let history = restored
            .read_application_conversation("page", "app")
            .unwrap();
        assert_eq!(history["messages"].as_array().unwrap().len(), 20);
        assert_eq!(history["messages"][19]["prompt"], "原文20");
        assert_eq!(history["messages"][19]["run"]["output"], "回复20");
        {
            let state = lock(&restored.inner);
            let events = &state.application_model_runs["app_run_20"].events;
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].item_id.as_deref(), Some("answer20"));
            assert_eq!(events[0].status, "completed");
        }
        assert!(history["messages"][19]["run"].get("threadId").is_none());
        assert!(
            restored
                .read_application_conversation("page", "other")
                .is_err()
        );
        {
            let mut state = lock(&restored.inner);
            let cached = state.application_model_runs.get_mut("app_run_20").unwrap();
            cached.snapshot.status = "failed".into();
            cached.snapshot.output = Some("过时的缓存".into());
            cached.snapshot.error_reason = Some("过时的传输失败".into());
            cached.snapshot.error_code = Some("model_stale_transport_error".into());
        }
        let refreshed = restored
            .read_application_conversation("page", "app")
            .unwrap();
        assert_eq!(refreshed["messages"][19]["run"]["output"], "回复20");
        assert_eq!(refreshed["messages"][19]["run"]["status"], "completed");
        assert!(refreshed["messages"][19]["run"]["errorReason"].is_null());
        assert!(refreshed["messages"][19]["run"]["errorCode"].is_null());
        assert!(lock(&restored.inner).persisted.conversations.is_empty());
        let metadata =
            AiRuntimeManager::application_store(&server, "read", &json!({"sessionId":"page"}))
                .unwrap();
        assert!(!metadata.to_string().contains("回复20"));
        assert_eq!(metadata["pendingAttachments"].as_array().unwrap().len(), 1);
        assert_eq!(
            metadata["pendingAttachments"][0]["attachmentId"],
            pending.attachment_id
        );
        assert!(PathBuf::from(&accepted_path).exists());
        restored.close_application_conversation("page").unwrap();
        assert!(!PathBuf::from(pending_path).exists());
        assert!(!PathBuf::from(accepted_path).exists());
        assert!(!service.catalog().unwrap().contains_key(native));
        assert_eq!(
            AiRuntimeManager::application_store(&server, "read", &json!({"sessionId":"page"}))
                .unwrap()["session"]["status"],
            "cleaned"
        );
        assert!(
            restored
                .open_application_conversation("page", "instance", "app", "authorization")
                .is_err()
        );
        assert!(lock(&restored.inner).persisted.conversations.is_empty());
        scoped_history.stop();
        stop_test_manager(&manager);
        stop_test_manager(&restored);
        server.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn an_undispatched_reservation_restores_as_not_accepted_and_can_close() {
        let _guard = crate::supervisor::app_server_test_guard();
        let root = super::super::model_test_fixture::fixture_root("app-reservation");
        let data = super::super::model_test_fixture::profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&data, &server).unwrap();
        manager
            .open_application_conversation("page", "instance", "app", "authorization")
            .unwrap();
        let snapshot = json!({"runId":"app_run_one","applicationId":"app","modelId":"model","modelLabel":"模型","status":"queued","startedAt":1000});
        AiRuntimeManager::application_store(&server,"reserve",&json!({"sessionId":"page","clientMessageId":"message","contentDigest":"digest","runId":"app_run_one","snapshot":snapshot,"context":{},"attachments":[]})).unwrap();
        let restored = AiRuntimeManager::new();
        restored.initialize(&data, &server).unwrap();
        let history = restored
            .read_application_conversation("page", "app")
            .unwrap();
        assert_eq!(history["messages"][0]["acceptance"], "not_accepted");
        assert_eq!(history["recoveryRequired"], false);
        assert!(history["messages"][0]["prompt"].is_null());
        assert!(history["messages"][0]["run"]["output"].is_null());
        restored.close_application_conversation("page").unwrap();
        assert_eq!(
            AiRuntimeManager::application_store(&server, "read", &json!({"sessionId":"page"}))
                .unwrap()["session"]["status"],
            "cleaned"
        );
        stop_test_manager(&manager);
        stop_test_manager(&restored);
        server.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn closed_native_completed_turn_reconciles_stale_run_metadata_before_cleanup() {
        assert_closed_cleanup(false);
    }
    #[test]
    fn cleanup_continues_after_native_deletion_without_reading_deleted_history() {
        assert_closed_cleanup(true);
    }
    fn assert_closed_cleanup(interrupt: bool) {
        let _guard = crate::supervisor::app_server_test_guard();
        let root = super::super::model_test_fixture::fixture_root("app-closed");
        let data = super::super::model_test_fixture::profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&data, &server).unwrap();
        manager
            .open_application_conversation("page", "instance", "app", "authorization")
            .unwrap();
        let scoped_history = ScopedHistory::new(&manager, &server);
        let service = std::sync::Arc::clone(&scoped_history.service);
        let created = service
            .request(
                "thread/start",
                &json!({"approvalPolicy":"never","sandbox":"read-only","ephemeral":false}),
            )
            .unwrap();
        let native = created["thread"]["id"].as_str().unwrap();
        service.import_history(&json!({"threadId":native,"name":"应用临时对话","turns":[{"id":"turn","startedAt":1,"completedAt":2,"error":null,"messages":[{"id":"message","role":"user","text":"原文"},{"id":"answer","role":"assistant","text":"回复"}]}]})).unwrap();
        let snapshot = json!({"runId":"app_run_one","applicationId":"app","modelId":"model","modelLabel":"模型","status":"running","startedAt":1000});
        AiRuntimeManager::application_store(&server,"reserve",&json!({"sessionId":"page","clientMessageId":"message","contentDigest":"digest","runId":"app_run_one","snapshot":snapshot,"context":{},"attachments":[]})).unwrap();
        let mut plan = conversation_kernel::thread_plan(
            server.clone(),
            ThreadInput {
                application_scope: Some(contexthub_protocol::AiThreadApplicationScope {
                    application_id: "app".into(),
                    session_kind: contexthub_protocol::AiApplicationSessionKind::ApplicationPage,
                }),
                conversation_id: "application:page".into(),
                run_id: "app_run_one".into(),
                message_id: "message".into(),
                client_message_id: "message".into(),
                content_digest: "digest".into(),
            },
            None,
            None,
        );
        scoped_history.bind(&mut plan);
        plan.journal.prepare("thread", "create", json!({})).unwrap();
        plan.journal
            .record("thread", "dispatch", Value::Null)
            .unwrap();
        plan.journal
            .record("thread", "acknowledge", json!({"nativeThreadId":native}))
            .unwrap();
        plan.journal
            .prepare_input("message", "message", "digest", false, "原文")
            .unwrap();
        plan.journal
            .accept_input("message", native, "turn")
            .unwrap();
        service.shutdown_checked().unwrap();
        plan.journal.release().unwrap();
        AiRuntimeManager::application_store(&server, "close", &json!({"sessionId":"page"}))
            .unwrap();
        if interrupt {
            manager
                .read_application_conversation("page", "app")
                .unwrap();
            AiRuntimeManager::application_store(
                &server,
                "beginCleanup",
                &json!({"sessionId":"page"}),
            )
            .unwrap();
            service
                .request("thread/delete", &json!({"threadId":native}))
                .unwrap();
        }
        let restored = AiRuntimeManager::new();
        restored.initialize(&data, &server).unwrap();
        assert_eq!(
            AiRuntimeManager::application_store(&server, "read", &json!({"sessionId":"page"}))
                .unwrap()["session"]["status"],
            "cleaned"
        );
        assert!(!service.catalog().unwrap().contains_key(native));
        scoped_history.stop();
        stop_test_manager(&manager);
        stop_test_manager(&restored);
        server.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
