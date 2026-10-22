use contexthub_platform_adapter::security;
use contexthub_protocol::PersonalProfileManifest as Manifest;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const APPLICATION_ID: &str = "cn.cilai.personal.workbench";
const PROFILE_FILE: &str = "local-profile.json";

pub struct LocalProfile {
    pub schema_version: u32,
    pub profile_id: String,
    pub display_name: String,
    pub directory: PathBuf,
}

pub fn validate_data_directory(directory: &Path) -> Result<(), String> {
    if directory.file_name().and_then(|v| v.to_str()) != Some("business")
        || directory
            .parent()
            .and_then(Path::file_name)
            .and_then(|v| v.to_str())
            != Some(APPLICATION_ID)
    {
        return Err("personal_data_directory_invalid".into());
    }
    validate_path(directory)
}

fn validate_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("local_profile_path_invalid".into());
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("local_profile_path_denied".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("local_profile_path_unavailable:{error}")),
        }
    }
    Ok(())
}

pub fn load_or_create(root: &Path) -> Result<LocalProfile, String> {
    validate_path(root)?;
    security::ensure_restricted_directory(root)
        .map_err(|error| format!("local_profile_directory_unavailable:{error}"))?;
    let path = root.join(PROFILE_FILE);
    let mut created = false;
    let manifest = match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
                return Err("local_profile_manifest_invalid".into());
            }
            security::verify_current_user_private_file(&path)
                .map_err(|_| "local_profile_manifest_permissions_invalid")?;
            let bytes =
                fs::read(&path).map_err(|error| format!("local_profile_read_failed:{error}"))?;
            serde_json::from_slice::<Manifest>(&bytes)
                .map_err(|_| "local_profile_manifest_invalid")?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if fs::read_dir(root)
                .map_err(|error| format!("local_profile_read_failed:{error}"))?
                .next()
                .is_some()
            {
                return Err("local_profile_manifest_missing_existing_data".into());
            }
            created = true;
            let manifest = Manifest {
                schema_version: 1,
                profile_id: uuid::Uuid::new_v4().simple().to_string(),
                display_name: "本地资料".into(),
            };
            let bytes = serde_json::to_vec(&manifest).map_err(|_| "local_profile_encode_failed")?;
            let mut file = security::create_new_restricted_file(&path)
                .map_err(|error| format!("local_profile_create_failed:{error}"))?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("local_profile_write_failed:{error}"))?;
            manifest
        }
        Err(error) => return Err(format!("local_profile_read_failed:{error}")),
    };
    let id = uuid::Uuid::parse_str(&manifest.profile_id)
        .map_err(|_| "local_profile_identifier_invalid")?;
    if !matches!(
        manifest.schema_version,
        1 | contexthub_protocol::PERSONAL_DATA_VERSION
    ) || id.get_version_num() != 4
        || id.simple().to_string() != manifest.profile_id
    {
        return Err("local_profile_manifest_version_invalid".into());
    }
    let directory = root.join("profiles").join(&manifest.profile_id);
    validate_path(&directory)?;
    if !created && !directory.is_dir() {
        return Err("local_profile_directory_missing".into());
    }
    security::ensure_restricted_directory(&directory)
        .map_err(|error| format!("local_profile_directory_unavailable:{error}"))?;
    Ok(LocalProfile {
        schema_version: manifest.schema_version,
        profile_id: manifest.profile_id,
        display_name: manifest.display_name,
        directory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("cilai-personal-profile-{}", uuid::Uuid::new_v4()))
            .join(APPLICATION_ID)
            .join("business")
    }

    fn clean(root: &Path) {
        fs::remove_dir_all(root.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn profile_reopens_without_model_or_enterprise_identity() {
        let root = root();
        validate_data_directory(&root).unwrap();
        let first = load_or_create(&root).unwrap();
        fs::write(
            first.directory.join("personal-material.txt"),
            "保留的个人材料",
        )
        .unwrap();
        let reopened = load_or_create(&root).unwrap();
        assert_eq!(first.profile_id, reopened.profile_id);
        assert_eq!(first.directory, reopened.directory);
        assert_eq!(
            fs::read_to_string(reopened.directory.join("personal-material.txt")).unwrap(),
            "保留的个人材料"
        );
        let manifest = fs::read_to_string(root.join(PROFILE_FILE)).unwrap();
        assert!(!manifest.contains("tenant"));
        assert!(!manifest.contains("account"));
        clean(&root);
    }

    #[test]
    fn missing_or_damaged_manifest_never_adopts_existing_material() {
        let root = root();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("contexthub.db"), "existing").unwrap();
        assert_eq!(
            load_or_create(&root).err().unwrap(),
            "local_profile_manifest_missing_existing_data"
        );
        assert!(!root.join(PROFILE_FILE).exists());
        fs::remove_file(root.join("contexthub.db")).unwrap();
        let profile = load_or_create(&root).unwrap();
        fs::write(root.join(PROFILE_FILE), "invalid").unwrap();
        assert_eq!(
            load_or_create(&root).err().unwrap(),
            "local_profile_manifest_invalid"
        );
        assert!(profile.directory.is_dir());
        clean(&root);
    }

    #[test]
    fn missing_owned_directory_is_not_recreated_as_empty_history() {
        let root = root();
        let profile = load_or_create(&root).unwrap();
        fs::remove_dir_all(&profile.directory).unwrap();
        assert_eq!(
            load_or_create(&root).err().unwrap(),
            "local_profile_directory_missing"
        );
        assert!(!profile.directory.exists());
        clean(&root);
    }

    #[test]
    fn enterprise_and_traversal_directories_are_rejected() {
        for path in [
            "/tmp/com.cilai.workbench/business",
            "/tmp/cn.cilai.personal.workbench/../business",
            "relative/cn.cilai.personal.workbench/business",
        ] {
            assert!(validate_data_directory(Path::new(path)).is_err());
        }
    }
}
