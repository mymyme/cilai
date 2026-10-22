use super::state::bounded_gate;
use super::{
    Actions, ApplicationActionCallback, Arc, BridgeWriter, Context, ContextId, DocumentEntry,
    DocumentHandle, HashSet, Invocation, Ordering, PersonalPageAiManager, PersonalPageAuditMethod,
    Publish, Value, id, json, lock, no_scope_receipt, now, parse,
};
use std::time::Duration;

pub(super) fn audit_method(method: &str) -> Option<PersonalPageAuditMethod> {
    Some(match method {
        "session.read" => PersonalPageAuditMethod::SessionRead,
        "context.publish" => PersonalPageAuditMethod::ContextPublish,
        "context.revoke" => PersonalPageAuditMethod::ContextRevoke,
        "uiAction.register" => PersonalPageAuditMethod::UiActionsRegister,
        "model.session.open" => PersonalPageAuditMethod::ModelSessionCreate,
        "model.session.read" => PersonalPageAuditMethod::ModelSessionRead,
        "model.run.create" => PersonalPageAuditMethod::ModelRunCreate,
        "model.run.get" => PersonalPageAuditMethod::ModelRunRead,
        "model.run.cancel" => PersonalPageAuditMethod::ModelRunCancel,
        "conversation.handoff" => PersonalPageAuditMethod::ConversationHandoffPrepare,
        _ => return None,
    })
}

pub(super) fn empty(params: &Value) -> Result<(), String> {
    if params.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(())
    } else {
        Err("personal_page_request_invalid".into())
    }
}

impl PersonalPageAiManager {
    pub(super) fn execute(
        &self,
        document: &str,
        doc: &DocumentHandle,
        request: &Invocation,
        ui_action: ApplicationActionCallback,
    ) -> Result<Value, String> {
        match request.method.as_str() {
            "session.read" => {
                empty(&request.params)?;
                Ok(Self::session_view(doc))
            }
            "context.publish" => self.publish(doc, &request.params),
            "context.revoke" => self.revoke_context(doc, &request.params),
            "uiAction.register" => self.register_actions(doc, &request.params),
            "model.session.open" => self.open_model_session(document, doc, &request.params),
            "model.session.read" => self.read_model_session(doc, &request.params),
            "model.run.create" => self.create_run(document, doc, &request.params, ui_action),
            "model.run.get" => self.read_run(doc, &request.params),
            "model.run.cancel" => self.cancel_run(doc, &request.params),
            "conversation.handoff" => self.prepare_handoff(doc, &request.params),
            _ => Err("personal_page_method_not_allowed".into()),
        }
    }

    fn publish(&self, doc: &DocumentHandle, params: &Value) -> Result<Value, String> {
        let p: Publish = parse(params)?;
        id(&p.context_id)?;
        if p.title.chars().count() > 200
            || p.summary.chars().count() > 2000
            || params.to_string().len() > 64 * 1024
        {
            return Err("personal_page_context_too_large".into());
        }
        let declaration = lock(doc)
            .binding
            .page_ai
            .context_types
            .iter()
            .find(|item| {
                item.context_type == p.context_type && item.schema_version == p.schema_version
            })
            .cloned()
            .ok_or("personal_page_context_not_declared")?;
        crate::app_bridge::validate_tool_arguments(&declaration.data_schema, &p.data)?;
        let full = {
            let scope = lock(doc);
            !scope.contexts.contains_key(&p.context_id) && scope.contexts.len() >= 64
        };
        if full {
            return Err("personal_page_context_limit".into());
        }
        let valid_until = now()?.saturating_add(600_000);
        self.change_revision(doc)?;
        let value = json!({"contextId":p.context_id,"contextType":p.context_type,"schemaVersion":p.schema_version,
            "title":p.title,"summary":p.summary,"data":p.data,"validUntil":valid_until});
        let mut scope = lock(doc);
        scope.contexts.insert(
            p.context_id,
            Context {
                expires_at: valid_until,
                value,
            },
        );
        Ok(json!({"pageRevision":scope.revision,"validUntil":valid_until}))
    }

    fn revoke_context(&self, doc: &DocumentHandle, params: &Value) -> Result<Value, String> {
        let p: ContextId = parse(params)?;
        if !lock(doc).contexts.contains_key(&p.context_id) {
            return Err("personal_page_context_missing".into());
        }
        self.change_revision(doc)?;
        let mut scope = lock(doc);
        scope.contexts.remove(&p.context_id);
        Ok(json!({"pageRevision":scope.revision}))
    }

    fn register_actions(&self, doc: &DocumentHandle, params: &Value) -> Result<Value, String> {
        let p: Actions = parse(params)?;
        let ids: HashSet<_> = p.action_ids.iter().cloned().collect();
        if ids.len() != p.action_ids.len() || ids.len() > 64 {
            return Err("personal_page_actions_invalid".into());
        }
        if ids.iter().any(|id| {
            !lock(doc)
                .binding
                .page_ai
                .ui_actions
                .iter()
                .any(|action| &action.action_id == id)
        }) {
            return Err("personal_page_action_not_declared".into());
        }
        self.change_revision(doc)?;
        let mut scope = lock(doc);
        scope.actions = ids;
        Ok(json!({"pageRevision":scope.revision,"actionIds":p.action_ids}))
    }

    fn change_revision(&self, doc: &DocumentHandle) -> Result<(), String> {
        {
            let mut scope = lock(doc);
            scope.revision = scope
                .revision
                .checked_add(1)
                .ok_or("personal_page_revision_exhausted")?;
        }
        self.retire_runs(doc)
    }

    pub(super) fn audit_list(&self, doc: &DocumentHandle, params: &Value) -> Result<Value, String> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Params {
            limit: u32,
            before_sequence: Option<i64>,
        }
        let p: Params = parse(params)?;
        self.ai.app_server()?.request_result("personalPage/auditList",&json!({
            "creationProjectId":lock(doc).binding.creation_project_id,"limit":p.limit,"beforeSequence":p.before_sequence
        }))
    }

    pub(super) fn retire_runs(&self, doc: &DocumentHandle) -> Result<(), String> {
        let ids = lock(doc).runs.iter().cloned().collect::<Vec<_>>();
        for id in &ids {
            let snapshot = self.ai.application_model_run(id)?;
            if matches!(
                snapshot.status.as_str(),
                "queued" | "running" | "waiting_confirmation" | "submitting"
            ) {
                self.ai.cancel_application_model_run(id)?;
            }
        }
        self.ai.await_personal_page_runs_exit(&ids)
    }

    /// # Errors
    /// 实际执行进程或已受理页面操作未确认退出时保留撤销状态并返回错误。
    pub fn close_document(&self, document: &str) -> Result<Value, String> {
        let entry = lock(&self.documents)
            .get(document)
            .cloned()
            .ok_or("personal_page_document_required")?;
        let gate = match entry {
            DocumentEntry::Retired(receipt) => return Ok(receipt),
            DocumentEntry::Registering(registration) => {
                registration.retired.store(true, Ordering::SeqCst);
                Arc::clone(&registration.gate)
            }
            DocumentEntry::Active(doc) => {
                let mut scope = lock(&doc);
                scope.retired = true;
                scope.cancellation.store(true, Ordering::SeqCst);
                Arc::clone(&scope.gate)
            }
        };
        let _serial = bounded_gate(&gate, Duration::from_secs(10))?;
        let entry = lock(&self.documents)
            .get(document)
            .cloned()
            .ok_or("personal_page_document_required")?;
        let doc = match entry {
            DocumentEntry::Retired(receipt) => return Ok(receipt),
            DocumentEntry::Registering(_) => {
                let receipt = no_scope_receipt();
                lock(&self.documents)
                    .insert(document.into(), DocumentEntry::Retired(receipt.clone()));
                return Ok(receipt);
            }
            DocumentEntry::Active(doc) => doc,
        };
        lock(&doc).retired = true;
        let mut receipt = self.close_active(document, &doc)?;
        let mut scope = lock(&doc);
        if scope.registration_unconfirmed {
            receipt["cleanupPending"] = json!(true);
            receipt["auditPending"] = json!(true);
        }
        scope.contexts.clear();
        scope.actions.clear();
        scope.requests.clear();
        scope.runs.clear();
        scope.model_sessions.clear();
        drop(scope);
        lock(&self.documents).insert(document.into(), DocumentEntry::Retired(receipt.clone()));
        Ok(receipt)
    }

    fn close_active(&self, document: &str, doc: &DocumentHandle) -> Result<Value, String> {
        self.retire_runs(doc)?;
        let operation = match self.ai.begin_capability_operation() {
            Ok(value) => Some(value),
            Err(error) if cleanup_deferred(&error) => None,
            Err(error) => return Err(error),
        };
        let writer = if operation.is_some() {
            match BridgeWriter::prepare(self, document, lock(doc).revision) {
                Ok(writer) => Some(writer),
                Err(error) if cleanup_deferred(&error) => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        let result = self.close_sessions(document, doc, writer.is_some());
        let result = if let Some(writer) = writer {
            writer.complete(result)
        } else {
            result
        }?;
        Ok(result)
    }

    fn close_sessions(
        &self,
        document: &str,
        doc: &DocumentHandle,
        admitted: bool,
    ) -> Result<Value, String> {
        let audit = admitted.then(|| {
            self.audit_start(
                document,
                doc,
                &uuid::Uuid::new_v4().simple().to_string(),
                PersonalPageAuditMethod::SessionClose,
                &json!({}),
                None,
            )
        });
        let mut cleanup_pending = false;
        for session in lock(doc).model_sessions.keys().cloned().collect::<Vec<_>>() {
            self.ai.mark_personal_page_conversation_closed(&session)?;
            if !admitted {
                cleanup_pending = true;
                continue;
            }
            match self.ai.cleanup_personal_page_conversation(&session) {
                Ok(false) => cleanup_pending = true,
                Ok(true) => {}
                Err(error) if cleanup_deferred(&error) => cleanup_pending = true,
                Err(error) => return Err(error),
            }
        }
        let audit_pending = match audit {
            Some(Ok(op)) => {
                self.audit_finish(&op, &Ok(json!({"retired":true,"processesExited":true})))?;
                false
            }
            _ => true,
        };
        Ok(
            json!({"retired":true,"processesExited":true,"cleanupPending":cleanup_pending,"auditPending":audit_pending}),
        )
    }

    /// # Errors
    /// 任一页面执行资源的退出不能确认时返回错误，不声明全部退出。
    pub fn revoke_all(&self) -> Result<Value, String> {
        let docs = lock(&self.documents).keys().cloned().collect::<Vec<_>>();
        let mut results = Vec::new();
        for doc in docs {
            results.push((doc.clone(), self.close_document(&doc)));
        }
        if results.iter().any(|(_, result)| result.is_err()) {
            return Err("personal_page_retirement_unconfirmed".into());
        }
        let cleanup = results
            .iter()
            .any(|(_, result)| result.as_ref().is_ok_and(|v| v["cleanupPending"] == true));
        let audit = results
            .iter()
            .any(|(_, result)| result.as_ref().is_ok_and(|v| v["auditPending"] == true));
        let documents = results
            .into_iter()
            .map(|(document, result)| {
                result.map(|receipt| json!({"documentId":document,"receipt":receipt}))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(
            json!({"retired":true,"processesExited":true,"cleanupPending":cleanup,"auditPending":audit,"documents":documents}),
        )
    }
}

fn cleanup_deferred(error: &str) -> bool {
    matches!(
        error,
        "personal_data_maintenance_active" | "ai_runtime_shutting_down"
    )
}
