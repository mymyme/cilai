use super::{
    ApplicationActionCallback, Arc, BridgeWriter, DocumentHandle, PersonalPageAiManager,
    PersonalPageAuditMethod, PersonalPageRiskLevel, RunCreate, RunId, ScopeValidator, SessionId,
    Value, digest, json, lock, operations, parse,
};
use crate::app_bridge::{ApplicationAiContextSnapshot, ApplicationUiActionSnapshot};

impl PersonalPageAiManager {
    pub(super) fn open_model_session(
        &self,
        document: &str,
        doc: &DocumentHandle,
        params: &Value,
    ) -> Result<Value, String> {
        operations::empty(params)?;
        if !lock(doc).binding.page_ai.model {
            return Err("personal_page_model_not_granted".into());
        }
        let selected =
            self.ai
                .personal_model_resolve(contexthub_protocol::ModelExecutionPrepareParams {
                    launch_protocol: 1,
                    owner: None,
                    purpose: contexthub_protocol::ModelPurpose::PageAi,
                    binding_id: None,
                    selection: None,
                    conversation_id: None,
                })?;
        let session = format!("personal_page_{}", uuid::Uuid::new_v4().simple());
        let (application, revision, authorization) = {
            let scope = lock(doc);
            (
                scope.binding.application_id.clone(),
                scope.revision,
                digest(
                    &json!({"binding":scope.binding,"document":document,"revision":scope.revision}),
                ),
            )
        };
        let opened = self.ai.open_application_conversation(
            &session,
            document,
            &application,
            &authorization,
        )?;
        lock(doc).model_sessions.insert(session.clone(), revision);
        Ok(
            json!({"modelSessionId":session,"pageRevision":revision,"conversation":opened,
            "modelId":selected.model_id,"connectionId":selected.connection_id}),
        )
    }

    pub(super) fn read_model_session(
        &self,
        doc: &DocumentHandle,
        params: &Value,
    ) -> Result<Value, String> {
        let p: SessionId = parse(params)?;
        let application = {
            let scope = lock(doc);
            if !scope.model_sessions.contains_key(&p.model_session_id) {
                return Err("personal_page_model_session_required".into());
            }
            scope.binding.application_id.clone()
        };
        self.ai
            .read_application_conversation(&p.model_session_id, &application)
    }

    pub(super) fn create_run(
        &self,
        document: &str,
        doc: &DocumentHandle,
        params: &Value,
        ui_action: ApplicationActionCallback,
    ) -> Result<Value, String> {
        let p: RunCreate = parse(params)?;
        let revision = lock(doc).revision;
        self.validate(doc, Some(revision))?;
        {
            let scope = lock(doc);
            if !scope.binding.page_ai.model {
                return Err("personal_page_model_not_granted".into());
            }
            if scope.model_sessions.get(&p.model_session_id) != Some(&revision) {
                return Err("personal_page_model_session_stale".into());
            }
        }
        let manager = self.clone();
        let handle = Arc::clone(doc);
        let validate: ScopeValidator = Arc::new(move || manager.validate(&handle, Some(revision)));
        let factory = self.callbacks(document, doc, ui_action);
        let context = Self::model_context(doc);
        let prompt = format!(
            "当前页面上下文：\n{}",
            serde_json::to_string(&context.contexts)
                .map_err(|_| "personal_page_context_invalid")?
        );
        let result = self.ai.start_page_model_run_impl(
            context,
            &p.model_id,
            &prompt,
            &p.prompt,
            &p.model_session_id,
            &p.client_message_id,
            &validate,
            &factory,
        )?;
        lock(doc).runs.insert(result.run_id.clone());
        self.monitor(doc, result.run_id.clone(), validate)?;
        serde_json::to_value(result).map_err(|_| "personal_page_response_invalid".into())
    }

    fn model_context(doc: &DocumentHandle) -> ApplicationAiContextSnapshot {
        let scope = lock(doc);
        ApplicationAiContextSnapshot {
            application_id: scope.binding.application_id.clone(),
            application_name: scope.binding.name.clone(),
            application_ids: vec![scope.binding.application_id.clone()],
            restrict_to_applications: true,
            skill_binding: None,
            model_task: None,
            browser_origins: vec![],
            report_review: false,
            local_material_paths: vec![],
            version: scope.binding.version.clone(),
            page_revision: scope.revision,
            contexts: scope
                .contexts
                .values()
                .map(|value| value.value.clone())
                .collect(),
            actions: scope
                .binding
                .page_ai
                .ui_actions
                .iter()
                .filter(|action| scope.actions.contains(&action.action_id))
                .map(|action| ApplicationUiActionSnapshot {
                    action_id: action.action_id.clone(),
                    title: action.title.clone(),
                    description: action.description.clone(),
                    input_schema: action.input_schema.clone(),
                    risk_level: match action.risk_level {
                        PersonalPageRiskLevel::Low => "low",
                        PersonalPageRiskLevel::Medium => "medium",
                        PersonalPageRiskLevel::High => "high",
                    }
                    .into(),
                    preconditions: action.preconditions.clone(),
                    expected_effects: action.expected_effects.clone(),
                })
                .collect(),
            tool_names: vec![],
            read_tools: vec![],
            write_tools: vec![],
            searchable_applications: vec![],
            tool_set_revision: None,
        }
    }

    fn monitor(
        &self,
        doc: &DocumentHandle,
        run: String,
        validate: ScopeValidator,
    ) -> Result<(), String> {
        let operation = match self.ai.begin_capability_operation() {
            Ok(operation) => operation,
            Err(error) => {
                self.ai.cancel_application_model_run(&run)?;
                self.ai.await_personal_page_runs_exit(&[run])?;
                return Err(error);
            }
        };
        let manager = self.clone();
        let handle = Arc::clone(doc);
        let run_id = run.clone();
        if std::thread::Builder::new()
            .name("personal-page-scope".into())
            .spawn(move || {
                let _operation = operation;
                while let Ok(snapshot) = manager.ai.application_model_run(&run_id) {
                    if !matches!(
                        snapshot.status.as_str(),
                        "queued"
                            | "running"
                            | "waiting_confirmation"
                            | "submitting"
                            | "terminating"
                    ) {
                        break;
                    }
                    if validate().is_err() {
                        if snapshot.status != "terminating"
                            && manager.ai.cancel_application_model_run(&run_id).is_err()
                        {
                            eprintln!("personal_page_scope_exit_unconfirmed");
                        }
                        if manager
                            .ai
                            .await_personal_page_runs_exit(std::slice::from_ref(&run_id))
                            .is_err()
                        {
                            lock(&handle).retired = true;
                        }
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            })
            .is_err()
        {
            self.ai.cancel_application_model_run(&run)?;
            self.ai.await_personal_page_runs_exit(&[run])?;
            return Err("personal_page_scope_monitor_unavailable".into());
        }
        Ok(())
    }

    fn owned_run(doc: &DocumentHandle, params: &Value) -> Result<String, String> {
        let p: RunId = parse(params)?;
        if !lock(doc).runs.contains(&p.run_id) {
            return Err("personal_page_run_owner_required".into());
        }
        Ok(p.run_id)
    }

    pub(super) fn read_run(&self, doc: &DocumentHandle, params: &Value) -> Result<Value, String> {
        serde_json::to_value(
            self.ai
                .application_model_run(&Self::owned_run(doc, params)?)?,
        )
        .map_err(|_| "personal_page_response_invalid".into())
    }

    pub(super) fn cancel_run(&self, doc: &DocumentHandle, params: &Value) -> Result<Value, String> {
        serde_json::to_value(
            self.ai
                .cancel_application_model_run(&Self::owned_run(doc, params)?)?,
        )
        .map_err(|_| "personal_page_response_invalid".into())
    }

    pub(super) fn prepare_handoff(
        &self,
        doc: &DocumentHandle,
        params: &Value,
    ) -> Result<Value, String> {
        if !lock(doc).binding.page_ai.conversation_handoff {
            return Err("personal_page_handoff_not_granted".into());
        }
        let application = lock(doc).binding.application_id.clone();
        self.ai
            .create_application_handoff(&application, &Self::owned_run(doc, params)?)
    }

    /// # Errors
    /// 仅供可信工作台确认界面调用；文档、运行或确认已失效时拒绝执行。
    pub fn resolve_write(
        &self,
        document: &str,
        run: &str,
        confirmation: &str,
        decision: &str,
        arguments: Option<&Value>,
    ) -> Result<Value, String> {
        let doc = self.document(document)?;
        self.validate(&doc, None)?;
        if !lock(&doc).runs.contains(run) {
            return Err("personal_page_run_owner_required".into());
        }
        let _operation = self.ai.begin_capability_operation()?;
        let writer = BridgeWriter::prepare(self, document, lock(&doc).revision)?;
        let result = (|| {
            let audit = self.audit_start(
                document,
                &doc,
                &uuid::Uuid::new_v4().simple().to_string(),
                PersonalPageAuditMethod::ModelWriteResolve,
                &json!({"confirmationId":confirmation,"decision":decision,"arguments":arguments}),
                Some(run.into()),
            )?;
            let result = self
                .ai
                .resolve_application_model_write(run, confirmation, decision, arguments)
                .and_then(|result| {
                    serde_json::to_value(result)
                        .map_err(|_| "personal_page_response_invalid".to_owned())
                });
            self.audit_finish(&audit, &result)?;
            result
        })();
        writer.complete(result)
    }
}
