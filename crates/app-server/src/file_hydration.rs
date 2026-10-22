use crate::{AppServerServices, ServiceError};
use contexthub_application::{UnitOfWork, WorkspaceRepository};
use contexthub_domain::{WorkspaceId, WorkspaceRelativePath};
use contexthub_platform_adapter::{cloud_hydration, filesystem};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

pub(crate) use contexthub_protocol::FileHydrationParams as Params;

struct Task {
    workspace_id: String,
    root: PathBuf,
    path: PathBuf,
    identity: String,
    size: u64,
    modified: Option<SystemTime>,
    started: Instant,
    request: Option<cloud_hydration::HydrationRequest>,
    terminal: Option<&'static str>,
}

#[derive(Default)]
pub(crate) struct HydrationTasks {
    tasks: HashMap<String, Task>,
}

impl HydrationTasks {
    fn active_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|task| {
                task.terminal.is_none() && task.started.elapsed() < Duration::from_mins(2)
            })
            .count()
    }
}
impl Task {
    fn finish(&mut self, state: &'static str) {
        self.terminal = Some(state);
        self.request = None;
    }
}

fn scoped_path(root: &Path, relative: &str) -> Result<PathBuf, ServiceError> {
    WorkspaceRelativePath::parse(relative)
        .map_err(|_| ServiceError::new("file_hydration_path_invalid"))?;
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|_| ServiceError::new("file_hydration_file_unavailable"))?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(ServiceError::new("file_hydration_scope_invalid"));
    }
    Ok(path)
}

impl AppServerServices {
    pub(crate) fn hydrate_file(
        &mut self,
        params: &Params,
        _now: i64,
    ) -> Result<Value, ServiceError> {
        let workspace = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .transact(|repo| repo.workspace(&WorkspaceId::new(params.workspace_id.clone())))
            .map_err(|_| ServiceError::new("workspace_state_failed"))?
            .ok_or_else(|| ServiceError::new("workspace_not_found"))?;
        if workspace.workspace_kind.as_str() != "local" {
            return Err(ServiceError::new("workspace_kind_unsupported"));
        }
        let root = PathBuf::from(workspace.local_root)
            .canonicalize()
            .map_err(|_| ServiceError::new("file_hydration_root_unavailable"))?;
        let path = scoped_path(&root, &params.relative_path)?;
        let snapshot = filesystem::metadata_with_stable_identity(&path)
            .map_err(|_| ServiceError::new("file_hydration_file_unavailable"))?;
        self.hydration_tasks
            .tasks
            .retain(|_, task| task.started.elapsed() < Duration::from_mins(5));
        if let Some(id) = params.operation_id.as_ref() {
            let task = self
                .hydration_tasks
                .tasks
                .get_mut(id)
                .ok_or_else(|| ServiceError::new("file_hydration_operation_expired"))?;
            if task.workspace_id != params.workspace_id
                || task.root != root
                || task.path != path
                || task.identity != snapshot.stable_identity
                || task.size != snapshot.metadata.len()
                || !cloud_hydration::same_modified_time(
                    task.modified,
                    snapshot.metadata.modified().ok(),
                )
            {
                task.finish("file_hydration_source_changed");
                return Err(ServiceError::new("file_hydration_source_changed"));
            }
            if params.cancel && task.terminal.is_none() {
                task.finish("cancelled");
            }
            if let Some(state) = task.terminal {
                return if matches!(state, "ready" | "cancelled") {
                    Ok(json!({"state":state,"operationId":id,"relativePath":params.relative_path}))
                } else {
                    Err(ServiceError::new(state))
                };
            }
            if task.started.elapsed() >= Duration::from_mins(2) {
                task.finish("file_hydration_timeout");
                return Err(ServiceError::new("file_hydration_timeout"));
            }
            if let Some(request) = task.request.as_mut()
                && let Err(error) = request.poll()
            {
                let reason = match error.kind() {
                    std::io::ErrorKind::Unsupported => "file_hydration_provider_unsupported",
                    _ => "file_hydration_download_failed",
                };
                task.finish(reason);
                return Err(ServiceError::new(reason));
            }
            let state = if filesystem::is_cloud_placeholder_file(&snapshot.metadata) {
                "downloading"
            } else {
                "ready"
            };
            if state == "ready" {
                task.finish("ready");
            }
            return Ok(json!({"state":state,"operationId":id,"relativePath":params.relative_path}));
        }
        if params.cancel {
            return Err(ServiceError::new("file_hydration_operation_required"));
        }
        if !filesystem::is_cloud_placeholder_file(&snapshot.metadata) {
            return Ok(json!({"state":"ready","relativePath":params.relative_path}));
        }
        if self.hydration_tasks.active_count() >= 32 {
            return Err(ServiceError::new("file_hydration_busy"));
        }
        let request = cloud_hydration::request_download(&path)
            .map_err(|_| ServiceError::new("file_hydration_busy"))?;
        let id = uuid::Uuid::new_v4().to_string();
        self.hydration_tasks.tasks.insert(
            id.clone(),
            Task {
                workspace_id: params.workspace_id.clone(),
                root,
                path,
                identity: snapshot.stable_identity,
                size: snapshot.metadata.len(),
                modified: snapshot.metadata.modified().ok(),
                started: Instant::now(),
                request: Some(request),
                terminal: None,
            },
        );
        Ok(json!({"state":"downloading","operationId":id,"relativePath":params.relative_path}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_cancelled_and_expired_hydrations_release_capacity() {
        let mut tasks = HydrationTasks::default();
        for index in 0..64 {
            let mut task = Task {
                workspace_id: String::new(),
                root: PathBuf::new(),
                path: PathBuf::new(),
                identity: String::new(),
                size: 0,
                modified: None,
                started: Instant::now(),
                request: None,
                terminal: None,
            };
            if index < 32 {
                task.finish(if index % 2 == 0 { "ready" } else { "cancelled" });
            } else {
                task.started = Instant::now()
                    .checked_sub(Duration::from_secs(121))
                    .unwrap();
            }
            tasks.tasks.insert(index.to_string(), task);
        }
        assert_eq!(tasks.active_count(), 0);
        let task = tasks.tasks.get_mut("0").unwrap();
        task.terminal = None;
        task.started = Instant::now();
        assert_eq!(tasks.active_count(), 1);
    }
    #[test]
    fn scope_rejects_parent_and_absolute_paths() {
        let root = std::env::temp_dir().join(format!("cilai-hydration-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("workspace")).unwrap();
        std::fs::write(root.join("outside.txt"), b"outside").unwrap();
        std::fs::write(root.join("workspace/local.txt"), b"inside").unwrap();
        let workspace = root.join("workspace").canonicalize().unwrap();
        assert!(scoped_path(&workspace, "local.txt").is_ok());
        assert!(scoped_path(&workspace, "../outside.txt").is_err());
        assert!(scoped_path(&workspace, "/outside.txt").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
