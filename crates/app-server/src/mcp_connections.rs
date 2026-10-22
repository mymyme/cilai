use crate::{AppServerServices, ServiceError};
use serde_json::Value;

impl AppServerServices {
    pub(crate) fn mcp_connection_rpc(
        &mut self,
        method: &str,
        params: Value,
        instance_fence: &str,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let profile_id = self
            .personal_profile_id()
            .ok_or_else(|| ServiceError::new("mcp_profile_not_ready"))?;
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .personal_mcp_rpc(method, params, &profile_id, instance_fence, now)
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("mcp_storage_operation_failed"))
            })
    }
}
