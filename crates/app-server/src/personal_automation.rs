use crate::{AppServerServices, ServiceError};
use contexthub_protocol::{
    ModelExecutionLease, PersonalAutomationAdmitParams, PersonalAutomationAdmitResult,
    PersonalAutomationBindParams, PersonalAutomationBindResult, PersonalAutomationFinishParams,
    PersonalAutomationFinishResult, PersonalAutomationListTriggersParams,
    PersonalAutomationPrepareExecutionParams, PersonalAutomationResolveInterruptedParams,
    PersonalAutomationTriggerPage,
};
use serde::{Serialize, de::DeserializeOwned};

impl AppServerServices {
    fn automation_call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: &P,
        now: i64,
    ) -> Result<R, ServiceError> {
        let profile = self
            .personal_profile_id()
            .ok_or_else(|| ServiceError::new("model_profile_not_ready"))?;
        let database = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let params = serde_json::to_value(params)
            .map_err(|_| ServiceError::new("automation_claim_request_invalid"))?;
        let result = database
            .personal_automation_rpc(method, params, &profile, now)
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("automation_claim_storage_failed"))
            })?;
        serde_json::from_value(result)
            .map_err(|_| ServiceError::new("automation_claim_record_invalid"))
    }

    pub(crate) fn personal_automation_admit(
        &mut self,
        p: &PersonalAutomationAdmitParams,
        now: i64,
    ) -> Result<PersonalAutomationAdmitResult, ServiceError> {
        self.automation_call("personalAutomation/admit", p, now)
    }
    pub(crate) fn personal_automation_prepare_execution(
        &mut self,
        p: &PersonalAutomationPrepareExecutionParams,
        now: i64,
    ) -> Result<ModelExecutionLease, ServiceError> {
        self.automation_call("personalAutomation/prepareExecution", p, now)
    }
    pub(crate) fn personal_automation_bind(
        &mut self,
        p: &PersonalAutomationBindParams,
        now: i64,
    ) -> Result<PersonalAutomationBindResult, ServiceError> {
        self.automation_call("personalAutomation/bind", p, now)
    }
    pub(crate) fn personal_automation_finish(
        &mut self,
        p: &PersonalAutomationFinishParams,
        now: i64,
    ) -> Result<PersonalAutomationFinishResult, ServiceError> {
        self.automation_call("personalAutomation/finish", p, now)
    }
    pub(crate) fn personal_automation_list(
        &mut self,
        p: &PersonalAutomationListTriggersParams,
        now: i64,
    ) -> Result<PersonalAutomationTriggerPage, ServiceError> {
        self.automation_call("personalAutomation/listTriggers", p, now)
    }
    pub(crate) fn personal_automation_resolve(
        &mut self,
        p: &PersonalAutomationResolveInterruptedParams,
        now: i64,
    ) -> Result<PersonalAutomationFinishResult, ServiceError> {
        self.automation_call("personalAutomation/resolveInterrupted", p, now)
    }
}
