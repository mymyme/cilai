use crate::{AppServerServices, ServiceError};
use contexthub_persistence_sqlite::{PersistenceError, SqlitePersistence};
use contexthub_protocol::{
    ModelUsageBeginParams, ModelUsageFinishParams, ModelUsageListParams, ModelUsageListResult,
    ModelUsageParams, ModelUsageRecord,
};

fn storage(error: &PersistenceError) -> ServiceError {
    ServiceError::new(error.reason().unwrap_or("model_usage_storage_failed"))
}
impl AppServerServices {
    fn usage_database(&mut self) -> Result<(String, &mut SqlitePersistence), ServiceError> {
        let profile = self
            .personal_profile_id()
            .ok_or_else(|| ServiceError::new("model_profile_not_ready"))?;
        let db = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        Ok((profile, db))
    }
    pub(crate) fn model_usage_begin(
        &mut self,
        p: &ModelUsageBeginParams,
        now: i64,
    ) -> Result<ModelUsageRecord, ServiceError> {
        let (profile, db) = self.usage_database()?;
        db.begin_model_usage(&profile, p, now)
            .map_err(|e| storage(&e))
    }
    pub(crate) fn model_usage_finish(
        &mut self,
        p: &ModelUsageFinishParams,
        now: i64,
    ) -> Result<ModelUsageRecord, ServiceError> {
        let (profile, db) = self.usage_database()?;
        db.finish_model_usage(&profile, p, now)
            .map_err(|e| storage(&e))
    }
    pub(crate) fn model_usage_get(
        &mut self,
        p: &ModelUsageParams,
        _: i64,
    ) -> Result<ModelUsageRecord, ServiceError> {
        let (profile, db) = self.usage_database()?;
        db.model_usage(&profile, &p.request_id)
            .map_err(|e| storage(&e))
    }
    pub(crate) fn model_usage_list(
        &mut self,
        p: &ModelUsageListParams,
        _: i64,
    ) -> Result<ModelUsageListResult, ServiceError> {
        let (profile, db) = self.usage_database()?;
        db.list_model_usage(&profile, p).map_err(|e| storage(&e))
    }
}
