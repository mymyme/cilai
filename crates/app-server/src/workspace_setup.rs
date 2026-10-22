use super::{AppServerServices, ServiceError};
use contexthub_domain::WorkspaceRelativePath;
use contexthub_protocol::{
    WorkspaceDirectoryCreateParams, WorkspaceDirectoryCreateResult, WorkspaceTargetValidateParams,
};
use std::{fs, io, path::Path};

pub(super) fn validate_directory_name(name: &str) -> Result<(), ServiceError> {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.trim().is_empty()
        || name != name.trim()
        || name.ends_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(ServiceError::new("directory_name_invalid"));
    }
    Ok(())
}

fn create_directory(root: &Path, parent: &str, name: &str) -> Result<String, ServiceError> {
    validate_directory_name(name)?;
    let root = root
        .canonicalize()
        .map_err(|_| ServiceError::new("workspace_root_unavailable"))?;
    let mut directory = root.clone();
    let parent = if parent.is_empty() {
        String::new()
    } else {
        WorkspaceRelativePath::parse(parent)
            .map_err(|_| ServiceError::new("workspace_directory_path_invalid"))?
            .as_str()
            .to_owned()
    };
    if !parent.is_empty() {
        for component in parent.split('/') {
            directory.push(component);
            let metadata = fs::symlink_metadata(&directory)
                .map_err(|_| ServiceError::new("workspace_directory_parent_unavailable"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ServiceError::new("workspace_directory_parent_invalid"));
            }
        }
    }
    let directory = directory
        .canonicalize()
        .map_err(|_| ServiceError::new("workspace_directory_parent_unavailable"))?;
    if !directory.starts_with(&root) {
        return Err(ServiceError::new("workspace_directory_path_invalid"));
    }
    fs::create_dir(directory.join(name)).map_err(|error| {
        ServiceError::new(if error.kind() == io::ErrorKind::AlreadyExists {
            "workspace_directory_already_exists"
        } else {
            "workspace_directory_create_failed"
        })
    })?;
    Ok(if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    })
}

impl AppServerServices {
    pub(super) fn workspace_directory_create(
        &mut self,
        params: &WorkspaceDirectoryCreateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<WorkspaceDirectoryCreateResult, ServiceError> {
        if params.client_operation_id.trim().is_empty() {
            return Err(ServiceError::new("invalid_workspace_operation_id"));
        }
        let target = self.workspace_target_validate(
            &WorkspaceTargetValidateParams {
                workspace_id: params.workspace_id.clone(),
                relative_path: None,
                require_write: true,
            },
            now,
        )?;
        let relative_path = create_directory(
            Path::new(&target.local_root),
            &params.parent_relative_path,
            &params.name,
        )?;
        Ok(WorkspaceDirectoryCreateResult { relative_path })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_directory_creation_uses_registered_workspace_without_adding_another() {
        let root =
            std::env::temp_dir().join(format!("cilai-directory-service-{}", uuid::Uuid::new_v4()));
        let data = root.join("data");
        let local = root.join("local");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir(&local).unwrap();
        let persistence =
            contexthub_persistence_sqlite::SqlitePersistence::open(&data.join("test.db"), "0.1.0")
                .unwrap();
        let mut services = AppServerServices::new(Some(persistence));
        let workspace = services
            .workspace_create(
                &contexthub_protocol::WorkspaceCreateParams {
                    local_root: local.to_str().unwrap().to_owned(),
                    display_name: "资料".to_owned(),
                    client_operation_id: "local-create".to_owned(),
                },
                "desktop",
                1,
            )
            .unwrap();
        let mut params = WorkspaceDirectoryCreateParams {
            workspace_id: workspace.workspace.workspace_id,
            parent_relative_path: String::new(),
            name: "新增目录".to_owned(),
            client_operation_id: "create-directory-1".to_owned(),
        };
        assert_eq!(
            services
                .workspace_directory_create(&params, "desktop", 2)
                .unwrap()
                .relative_path,
            "新增目录"
        );
        assert!(local.join("新增目录").is_dir());
        assert_eq!(
            services
                .workspace_list(&contexthub_protocol::WorkspaceListParams {})
                .unwrap()
                .workspaces
                .len(),
            1
        );
        params.workspace_id = "missing".to_owned();
        params.name = "不能写入".to_owned();
        assert_eq!(
            services
                .workspace_directory_create(&params, "desktop", 3)
                .unwrap_err()
                .reason,
            "workspace_not_found"
        );
        assert!(!local.join("不能写入").exists());
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_creation_stays_in_selected_parent_and_preserves_existing_items() {
        let root = std::env::temp_dir().join(format!("cilai-directory-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("资料")).unwrap();
        fs::write(root.join("资料/已有.txt"), "保留").unwrap();
        assert_eq!(
            create_directory(&root, "资料", "新文件夹").unwrap(),
            "资料/新文件夹"
        );
        assert!(root.join("资料/新文件夹").is_dir());
        assert_eq!(
            create_directory(&root, "资料", "新文件夹")
                .unwrap_err()
                .reason,
            "workspace_directory_already_exists"
        );
        assert!(create_directory(&root, "../outside", "目录").is_err());
        assert!(create_directory(&root, "资料/已有.txt", "目录").is_err());
        assert_eq!(
            fs::read_to_string(root.join("资料/已有.txt")).unwrap(),
            "保留"
        );
        assert_eq!(
            create_directory(&root, "", "根目录文件夹").unwrap(),
            "根目录文件夹"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
