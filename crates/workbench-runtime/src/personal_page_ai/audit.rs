use super::{
    DocumentHandle, PersonalPageAiManager, PersonalPageAuditFinishParams, PersonalPageAuditMethod,
    PersonalPageAuditOutcome, PersonalPageAuditPrepareParams, PersonalPageAuditRecord, Value,
    digest, lock, parse,
};

impl PersonalPageAiManager {
    pub(super) fn audit_start(
        &self,
        document: &str,
        scope: &DocumentHandle,
        request: &str,
        method: PersonalPageAuditMethod,
        args: &Value,
        run: Option<String>,
    ) -> Result<String, String> {
        let prepared = {
            let doc = lock(scope);
            PersonalPageAuditPrepareParams {
                operation_id: uuid::Uuid::new_v4().simple().to_string(),
                binding: doc.params.clone(),
                permission_digest: doc.binding.permission_digest.clone(),
                document_id: document.into(),
                session_id: Some(doc.session.clone()),
                run_id: run,
                request_id: request.into(),
                method,
                arguments_digest: digest(args),
            }
        };
        let response: PersonalPageAuditRecord = parse(&self.ai.app_server()?.request_result(
            "personalPage/auditPrepare",
            &serde_json::to_value(&prepared).map_err(|_| "personal_page_request_invalid")?,
        )?)?;
        if response.request != prepared
            || response.outcome.is_some()
            || response.read_only_after_restore
        {
            return Err("personal_page_operation_unconfirmed".into());
        }
        Ok(prepared.operation_id)
    }

    pub(super) fn audit_finish(
        &self,
        operation: &str,
        result: &Result<Value, String>,
    ) -> Result<(), String> {
        let (outcome, reason) = match result {
            Ok(value) if value["status"] == "failed" => (
                PersonalPageAuditOutcome::Failed,
                Some("personal_page_operation_failed"),
            ),
            Ok(_) => (PersonalPageAuditOutcome::Succeeded, None),
            Err(e) if e.ends_with("_rejected") => (
                PersonalPageAuditOutcome::Rejected,
                Some("personal_page_operation_rejected"),
            ),
            Err(e) if e.ends_with("_cancelled") => (
                PersonalPageAuditOutcome::Cancelled,
                Some("personal_page_operation_cancelled"),
            ),
            Err(e) if unknown(e) => (
                PersonalPageAuditOutcome::OutcomeUnknown,
                Some("personal_page_operation_unconfirmed"),
            ),
            Err(_) => (
                PersonalPageAuditOutcome::Failed,
                Some("personal_page_operation_failed"),
            ),
        };
        let request = PersonalPageAuditFinishParams {
            operation_id: operation.into(),
            outcome,
            reason_code: reason.map(str::to_owned),
        };
        let response: PersonalPageAuditRecord = (|| {
            parse(&self.ai.app_server()?.request_result(
                "personalPage/auditFinish",
                &serde_json::to_value(&request).map_err(|_| "personal_page_request_invalid")?,
            )?)
        })()
        .map_err(|_: String| "personal_page_audit_finish_unconfirmed".to_owned())?;
        if response.request.operation_id != operation
            || response.outcome.as_ref() != Some(&request.outcome)
            || response.finished_at.is_none()
        {
            return Err("personal_page_audit_finish_unconfirmed".into());
        }
        Ok(())
    }
}

pub(super) fn unknown(error: &str) -> bool {
    error.ends_with("_unconfirmed") || error.ends_with("_unknown") || error.ends_with("_timeout")
}
