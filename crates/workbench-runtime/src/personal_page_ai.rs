//! 已安装个人页面的临时 AI 范围，不建立企业或产品账号身份。
mod audit;
mod model;
mod operations;
mod state;
mod tools;
mod writer;

use crate::ai_runtime::AiRuntimeManager;
use crate::application_tool_adapter::ScopeValidator;
use crate::codex_harness::{ApplicationActionCallback, PersonalToolsCallback};
use contexthub_protocol::{
    PersonalPageAuditFinishParams, PersonalPageAuditMethod, PersonalPageAuditOutcome,
    PersonalPageAuditPrepareParams, PersonalPageAuditRecord, PersonalPageBindingParams,
    PersonalPageBindingSnapshot, PersonalPageRiskLevel, PersonalPageTool, PersonalWriterLease,
    PersonalWriterMode, PersonalWriterState,
};
use serde_json::{Value, json};
use state::{
    Actions, Context, ContextId, Document, DocumentEntry, DocumentHandle, Invocation, Publish,
    Registration, RequestReceipt, RunCreate, RunId, SessionId, digest, id, lock, no_scope_receipt,
    now, parse,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use writer::BridgeWriter;

pub(crate) type PageCallbacksFactory = Arc<
    dyn Fn(&str, ScopeValidator) -> (Option<PersonalToolsCallback>, ApplicationActionCallback)
        + Send
        + Sync,
>;

pub type UiActionExecutor = Arc<dyn Fn(&str, Value) -> Result<Value, String> + Send + Sync>;

#[derive(Clone)]
pub struct PersonalPageAiManager {
    ai: AiRuntimeManager,
    documents: Arc<Mutex<HashMap<String, DocumentEntry>>>,
}

impl PersonalPageAiManager {
    #[must_use]
    pub fn new(ai: AiRuntimeManager) -> Self {
        Self {
            ai,
            documents: Arc::default(),
        }
    }

    /// # Errors
    /// 文档凭证重复、个人包或权限不能核对时拒绝开启。
    pub fn register_document(
        &self,
        document: &str,
        request: PersonalPageBindingParams,
    ) -> Result<Value, String> {
        let registration = self.reserve_document(document)?;
        let _serial = lock(&registration.gate);
        if registration.retired.load(Ordering::SeqCst) {
            return Err("personal_page_scope_revoked".into());
        }
        let mut created = None;
        let result = self.register_owned(document, request, &registration, &mut created);
        let cancelled = registration.retired.load(Ordering::SeqCst);
        let entry = if let Some(doc) = created {
            {
                let mut scope = lock(&doc);
                scope.retired = cancelled || result.is_err();
                scope.registration_unconfirmed =
                    result.as_ref().is_err_and(|error| audit::unknown(error));
            }
            DocumentEntry::Active(doc)
        } else {
            DocumentEntry::Retired(no_scope_receipt())
        };
        lock(&self.documents).insert(document.into(), entry);
        if cancelled {
            return Err("personal_page_scope_revoked".into());
        }
        result
    }

    fn reserve_document(&self, document: &str) -> Result<Arc<Registration>, String> {
        id(document)?;
        let mut documents = lock(&self.documents);
        if documents.contains_key(document) {
            return Err("personal_page_document_reused".into());
        }
        if documents
            .values()
            .filter(|entry| !matches!(entry, DocumentEntry::Retired(_)))
            .count()
            >= 128
        {
            documents.insert(document.into(), DocumentEntry::Retired(no_scope_receipt()));
            return Err("personal_page_document_limit".into());
        }
        let registration = Arc::new(Registration {
            gate: Arc::default(),
            retired: Arc::new(false.into()),
        });
        documents.insert(
            document.into(),
            DocumentEntry::Registering(Arc::clone(&registration)),
        );
        Ok(registration)
    }

    fn register_owned(
        &self,
        document: &str,
        request: PersonalPageBindingParams,
        registration: &Registration,
        created: &mut Option<DocumentHandle>,
    ) -> Result<Value, String> {
        let _operation = self.ai.begin_capability_operation()?;
        let binding = self.read_binding(&request)?;
        if registration.retired.load(Ordering::SeqCst) {
            return Err("personal_page_scope_revoked".into());
        }
        let session = format!("page_{}", uuid::Uuid::new_v4().simple());
        let doc = Arc::new(Mutex::new(Document {
            gate: Arc::clone(&registration.gate),
            cancellation: Arc::clone(&registration.retired),
            params: request,
            binding,
            session,
            revision: 1,
            retired: false,
            registration_unconfirmed: false,
            contexts: BTreeMap::new(),
            actions: HashSet::new(),
            model_sessions: HashMap::new(),
            runs: HashSet::new(),
            requests: HashMap::new(),
        }));
        *created = Some(Arc::clone(&doc));
        let writer = match BridgeWriter::prepare(self, document, 1) {
            Ok(writer) => writer,
            Err(error) => {
                lock(&doc).retired = true;
                return Err(error);
            }
        };
        let result = (|| {
            let op = self.audit_start(
                document,
                &doc,
                "register",
                PersonalPageAuditMethod::SessionOpen,
                &json!({}),
                None,
            )?;
            let result = Ok(Self::session_view(&doc));
            self.audit_finish(&op, &result)?;
            result
        })();
        if result.is_err() {
            lock(&doc).retired = true;
        }
        writer.complete(result)
    }

    /// # Errors
    /// 调用方文档、声明、修订或操作回执不能核对时拒绝执行。
    pub fn dispatch(
        &self,
        document: &str,
        request: Value,
        ui_action: ApplicationActionCallback,
    ) -> Result<Value, String> {
        if serde_json::to_vec(&request)
            .map_err(|_| "personal_page_request_invalid")?
            .len()
            > 256 * 1024
        {
            return Err("personal_page_request_too_large".into());
        }
        let request: Invocation =
            serde_json::from_value(request).map_err(|_| "personal_page_request_invalid")?;
        id(&request.request_id)?;
        let doc = self.document(document)?;
        let gate = Arc::clone(&lock(&doc).gate);
        let _serial = lock(&gate);
        self.validate(&doc, None)?;
        if request.method == "model.write.resolve" {
            return Err("personal_page_confirmation_host_required".into());
        }
        match request.method.as_str() {
            "session.read" => {
                operations::empty(&request.params)?;
                return Ok(Self::session_view(&doc));
            }
            "model.run.get" => return self.read_run(&doc, &request.params),
            "audit.list" => return self.audit_list(&doc, &request.params),
            _ => {}
        }
        let hash = digest(&json!({"method":request.method,"params":request.params}));
        {
            let mut scope = lock(&doc);
            if let Some(receipt) = scope.requests.get(&request.request_id) {
                if receipt.digest != hash {
                    return Err("personal_page_request_reused".into());
                }
                return receipt
                    .result
                    .clone()
                    .unwrap_or_else(|| Err("personal_page_operation_unconfirmed".into()));
            }
            if scope.requests.len() >= 4096 {
                return Err("personal_page_request_limit".into());
            }
            scope.requests.insert(
                request.request_id.clone(),
                RequestReceipt {
                    digest: hash,
                    result: None,
                },
            );
        }
        let result = self.dispatch_owned(document, &doc, &request, ui_action);
        lock(&doc)
            .requests
            .get_mut(&request.request_id)
            .ok_or("personal_page_request_missing")?
            .result = Some(result.clone());
        result
    }

    fn dispatch_owned(
        &self,
        document: &str,
        doc: &DocumentHandle,
        request: &Invocation,
        ui_action: ApplicationActionCallback,
    ) -> Result<Value, String> {
        let _operation = self.ai.begin_capability_operation()?;
        let writer = BridgeWriter::prepare(self, document, lock(doc).revision)?;
        let result = (|| {
            let Some(method) = operations::audit_method(&request.method) else {
                if request.method == "audit.list" {
                    let result = self.audit_list(doc, &request.params);
                    return result;
                }
                return Err("personal_page_method_not_allowed".into());
            };
            let audit = self.audit_start(
                document,
                doc,
                &request.request_id,
                method,
                &request.params,
                None,
            )?;
            let result = self.execute(document, doc, request, ui_action);
            if self.audit_finish(&audit, &result).is_err() {
                lock(doc).retired = true;
                return Err("personal_page_audit_finish_unconfirmed".into());
            }
            if result.as_ref().is_err_and(|e| audit::unknown(e)) {
                lock(doc).retired = true;
            }
            result
        })();
        writer.complete(result)
    }

    fn document(&self, document: &str) -> Result<DocumentHandle, String> {
        match lock(&self.documents).get(document) {
            Some(DocumentEntry::Active(doc)) => Ok(Arc::clone(doc)),
            Some(DocumentEntry::Registering(_)) => Err("personal_page_document_registering".into()),
            Some(DocumentEntry::Retired(_)) => Err("personal_page_scope_revoked".into()),
            None => Err("personal_page_document_required".into()),
        }
    }

    fn read_binding(
        &self,
        params: &PersonalPageBindingParams,
    ) -> Result<PersonalPageBindingSnapshot, String> {
        parse(&self.ai.app_server()?.request_result(
            "personalPage/binding",
            &serde_json::to_value(params).map_err(|_| "personal_page_request_invalid")?,
        )?)
    }

    fn validate(&self, doc: &DocumentHandle, revision: Option<u64>) -> Result<(), String> {
        let (params, binding) = {
            let scope = lock(doc);
            if scope.retired || scope.cancellation.load(Ordering::SeqCst) {
                return Err("personal_page_scope_revoked".into());
            }
            if revision.is_some_and(|v| v != scope.revision) {
                return Err("personal_page_revision_changed".into());
            }
            if revision.is_some()
                && scope
                    .contexts
                    .values()
                    .any(|context| context.expires_at <= now().unwrap_or(i64::MAX))
            {
                return Err("personal_page_context_expired".into());
            }
            (scope.params.clone(), scope.binding.clone())
        };
        if self.read_binding(&params)? != binding {
            lock(doc).retired = true;
            return Err("personal_page_binding_changed".into());
        }
        Ok(())
    }

    fn session_view(doc: &DocumentHandle) -> Value {
        let scope = lock(doc);
        json!({"sessionId":scope.session,"applicationId":scope.binding.application_id,
            "applicationName":scope.binding.name,"version":scope.binding.version,"pageRevision":scope.revision,
            "packageDigest":scope.binding.package_digest,"permissionDigest":scope.binding.permission_digest,
            "contextTypes":scope.binding.page_ai.context_types,"uiActions":scope.binding.page_ai.ui_actions,
            "model":scope.binding.page_ai.model,"conversationHandoff":scope.binding.page_ai.conversation_handoff,
            "readTools":scope.binding.page_ai.read_tools,"writeTools":scope.binding.page_ai.write_tools})
    }
}
