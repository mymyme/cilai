use super::{
    AppServerServices, PersonalPageAiManifest, ServiceError, persistence_error, validate_project_id,
};
use contexthub_protocol::{
    PersonalPageAuditFinishParams, PersonalPageAuditListParams, PersonalPageAuditListResult,
    PersonalPageAuditMethod as Method, PersonalPageAuditPrepareParams, PersonalPageAuditRecord,
};

fn authorized(method: &Method, manifest: &PersonalPageAiManifest) -> bool {
    match method {
        Method::SessionOpen | Method::SessionRead | Method::SessionClose => true,
        Method::ContextPublish | Method::ContextRevoke => !manifest.context_types.is_empty(),
        Method::UiActionsRegister | Method::UiActionInvoke => !manifest.ui_actions.is_empty(),
        Method::ModelSessionCreate
        | Method::ModelSessionRead
        | Method::ModelMessageRead
        | Method::ModelRunCreate
        | Method::ModelRunRead
        | Method::ModelRunCancel
        | Method::ModelWriteResolve => manifest.model,
        Method::ConversationHandoffPrepare | Method::ConversationHandoffCommit => {
            manifest.conversation_handoff
        }
        Method::ToolRead => !manifest.read_tools.is_empty(),
        Method::ToolWrite => !manifest.write_tools.is_empty(),
    }
}

impl AppServerServices {
    pub(crate) fn personal_page_audit_prepare(
        &mut self,
        p: &PersonalPageAuditPrepareParams,
        now: i64,
    ) -> Result<PersonalPageAuditRecord, ServiceError> {
        let binding = self.personal_page_binding(&p.binding, now)?;
        if !authorized(&p.method, &binding.page_ai) {
            return Err(ServiceError::new("personal_page_ai_not_authorized"));
        }
        self.persistence_mut()?
            .prepare_personal_page_audit(&binding.profile_id, p, &binding, now)
            .map_err(persistence_error)
    }

    pub(crate) fn personal_page_audit_finish(
        &mut self,
        p: &PersonalPageAuditFinishParams,
        now: i64,
    ) -> Result<PersonalPageAuditRecord, ServiceError> {
        let (_, profile) = self.data_context()?;
        self.persistence_mut()?
            .finish_personal_page_audit(&profile, p, now)
            .map_err(persistence_error)
    }

    pub(crate) fn personal_page_audit_list(
        &mut self,
        p: &PersonalPageAuditListParams,
        _: i64,
    ) -> Result<PersonalPageAuditListResult, ServiceError> {
        validate_project_id(&p.creation_project_id)?;
        let (_, profile) = self.data_context()?;
        self.persistence()?
            .personal_page_audits(&profile, p)
            .map_err(persistence_error)
    }
}
