use crate::{AppServerServices, ServiceError};
use serde_json::Value;

impl AppServerServices {
    pub(crate) fn personal_profile_id(&self) -> Option<String> {
        let data = self.data_dir.as_deref()?;
        let id = data.file_name()?.to_str()?;
        let profiles = data.parent()?;
        if id.len() != 32
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || profiles.file_name()?.to_str()? != "profiles"
            || profiles.parent()?.file_name()?.to_str()? != "business"
            || profiles.parent()?.parent()?.file_name()?.to_str()? != "cn.cilai.personal.workbench"
        {
            return None;
        }
        Some(id.into())
    }

    pub(crate) fn model_connection_rpc(
        &mut self,
        method: &str,
        params: Value,
        instance_fence: &str,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let profile_id = self
            .personal_profile_id()
            .ok_or_else(|| ServiceError::new("model_profile_not_ready"))?;
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .personal_model_rpc(method, params, &profile_id, instance_fence, now)
            .map_err(|e| ServiceError::new(e.reason().unwrap_or("model_storage_operation_failed")))
    }
}
