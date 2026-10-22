use super::{
    PersonalPageAiManager, PersonalWriterLease, PersonalWriterMode, PersonalWriterState, Value,
    audit, digest, json, parse,
};

pub(super) struct BridgeWriter {
    server: crate::supervisor::AppServerSupervisor,
    lease: PersonalWriterLease,
}

impl BridgeWriter {
    pub fn prepare(
        manager: &PersonalPageAiManager,
        document: &str,
        revision: u64,
    ) -> Result<Self, String> {
        let server = manager.ai.app_server()?;
        let lease: PersonalWriterLease = parse(&server.request_result("profileWriter/prepare", &json!({
            "clientOperationId":format!("personal-page-{}",uuid::Uuid::new_v4().simple()),
            "kind":"application_bridge","resourceId":digest(&json!({"document":document})),
            "resourceRevision":revision.to_string(),"owner":crate::model_connections::execution_owner()?,
            "launchProtocol":1,"processMode":"remote_request"
        }))?)?;
        if lease.state != PersonalWriterState::Active
            || lease.process_mode != PersonalWriterMode::RemoteRequest
        {
            return Err("personal_page_writer_unconfirmed".into());
        }
        Ok(Self { server, lease })
    }

    pub fn finish(self) -> Result<(), String> {
        let result: PersonalWriterLease = parse(&self.server.request_result(
            "profileWriter/release",
            &json!({
                "leaseId":self.lease.lease_id,"owner":self.lease.owner,
                "localTransportFinished":true,"remoteOutcomeUnconfirmed":false
            }),
        )?)?;
        if result.state != PersonalWriterState::Released {
            return Err("personal_page_writer_unconfirmed".into());
        }
        Ok(())
    }

    pub fn complete(self, result: Result<Value, String>) -> Result<Value, String> {
        if !result.as_ref().is_err_and(|error| audit::unknown(error)) {
            self.finish()?;
        }
        result
    }
}
