use crate::{AppServerServices, ServiceError, SharedAppServerServices, unix_epoch_millis};
use contexthub_application::{UnitOfWork, WorkspaceRepository};
use contexthub_domain::WorkspaceId;
use contexthub_protocol::{
    WorkspaceFilesParams, WorkspaceFilesResult, WorkspaceScanParams, WorkspaceScanResult,
};

pub(crate) struct Prepared {
    workspace_id: WorkspaceId,
}

impl AppServerServices {
    pub(crate) fn prepare_workspace_files(
        &mut self,
        params: &WorkspaceFilesParams,
        now: i64,
    ) -> Result<Prepared, ServiceError> {
        let limit = params.limit.unwrap_or(200);
        if limit == 0 || limit > crate::MAX_PAGE_SIZE {
            return Err(ServiceError::new("workspace_files_page_size_invalid"));
        }
        self.prepare_workspace_scan(
            &WorkspaceScanParams {
                workspace_id: params.workspace_id.clone(),
            },
            now,
        )
    }

    pub(crate) fn prepare_workspace_scan(
        &mut self,
        params: &WorkspaceScanParams,
        _now: i64,
    ) -> Result<Prepared, ServiceError> {
        let workspace_id = WorkspaceId::new(&params.workspace_id);
        let workspace = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .transact(|repositories| repositories.workspace(&workspace_id))
            .map_err(|_| ServiceError::new("workspace_scan_state_failed"))?
            .ok_or_else(|| ServiceError::new("workspace_not_found"))?;
        if workspace.workspace_kind.as_str() != "local" {
            return Err(ServiceError::new("personal_cloud_source_unavailable"));
        }
        Ok(Prepared { workspace_id })
    }
}

pub(crate) fn execute(
    prepared: &Prepared,
    params: &WorkspaceScanParams,
    services: &SharedAppServerServices,
) -> Result<WorkspaceScanResult, ServiceError> {
    if prepared.workspace_id.as_str() != params.workspace_id {
        return Err(ServiceError::new("workspace_request_changed"));
    }
    services
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_scan(params, unix_epoch_millis().map_err(ServiceError::new)?)
}

pub(crate) fn execute_files(
    prepared: &Prepared,
    params: &WorkspaceFilesParams,
    services: &SharedAppServerServices,
) -> Result<WorkspaceFilesResult, ServiceError> {
    if prepared.workspace_id.as_str() != params.workspace_id {
        return Err(ServiceError::new("workspace_request_changed"));
    }
    services
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .workspace_files(params)
}
