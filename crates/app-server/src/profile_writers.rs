use crate::{AppServerServices, ServiceError};
use serde_json::Value;

impl AppServerServices {
    pub(crate) fn profile_writer_rpc(
        &mut self,
        method: &str,
        params: Value,
        instance_fence: &str,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let profile = self
            .personal_profile_id()
            .ok_or_else(|| ServiceError::new("personal_writer_profile_not_ready"))?;
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .personal_writer_rpc(method, params, &profile, instance_fence, now)
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("personal_writer_storage_failed"))
            })
    }
}
