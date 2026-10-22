use contexthub_persistence_sqlite::SqlitePersistence;
use contexthub_platform_adapter::security;
use contexthub_protocol::{PERSONAL_DATA_VERSION, PersonalProfileManifest};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct UpgradeTarget {
    path: PathBuf,
    manifest: PersonalProfileManifest,
}

impl UpgradeTarget {
    pub fn read(data: &Path) -> Result<Option<Self>, String> {
        let Some(profiles) = data
            .parent()
            .filter(|p| p.file_name().is_some_and(|v| v == "profiles"))
        else {
            return Ok(None);
        };
        let Some(root) = profiles.parent().filter(|p| {
            p.file_name().is_some_and(|v| v == "business")
                && p.parent().is_some_and(|p| {
                    p.file_name()
                        .is_some_and(|v| v == "cn.cilai.personal.workbench")
                })
        }) else {
            return Ok(None);
        };
        for ancestor in data.ancestors() {
            let meta =
                fs::symlink_metadata(ancestor).map_err(|_| "local_profile_path_unavailable")?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err("local_profile_path_denied".into());
            }
        }
        let path = root.join("local-profile.json");
        let meta = fs::symlink_metadata(&path)
            .map_err(|_| "local_profile_manifest_missing_existing_data")?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 4096 {
            return Err("local_profile_manifest_invalid".into());
        }
        security::verify_current_user_private_file(&path)
            .map_err(|_| "local_profile_manifest_permissions_invalid")?;
        let manifest: PersonalProfileManifest =
            serde_json::from_slice(&fs::read(&path).map_err(|_| "local_profile_read_failed")?)
                .map_err(|_| "local_profile_manifest_invalid")?;
        let id = uuid::Uuid::parse_str(&manifest.profile_id)
            .map_err(|_| "local_profile_identifier_invalid")?;
        if id.get_version_num() != 4
            || id.simple().to_string() != manifest.profile_id
            || !matches!(manifest.schema_version, 1 | PERSONAL_DATA_VERSION)
            || data.file_name().and_then(|v| v.to_str()) != Some(manifest.profile_id.as_str())
        {
            return Err("local_profile_manifest_version_invalid".into());
        }
        Ok(Some(Self { path, manifest }))
    }

    /// 调用者必须在整个升级和服务生命周期持有同资料的 `InstanceGuard`。
    pub fn activate(
        mut self,
        persistence: &mut SqlitePersistence,
        instance_fence: &str,
    ) -> Result<(), String> {
        persistence
            .begin_personal_profile_upgrade(&self.manifest.profile_id, instance_fence)
            .map_err(|e| e.to_string())?;
        if self.manifest.schema_version != PERSONAL_DATA_VERSION {
            let original = fs::read(&self.path).map_err(|_| "local_profile_read_failed")?;
            let checked: PersonalProfileManifest =
                serde_json::from_slice(&original).map_err(|_| "local_profile_manifest_invalid")?;
            if checked != self.manifest {
                return Err("local_profile_changed_during_upgrade".into());
            }
            self.manifest.schema_version = PERSONAL_DATA_VERSION;
            let bytes =
                serde_json::to_vec(&self.manifest).map_err(|_| "local_profile_encode_failed")?;
            let temp = self.path.with_file_name(format!(
                "local-profile.json.upgrade-{}",
                uuid::Uuid::new_v4().simple()
            ));
            let mut file = security::create_new_restricted_file(&temp)
                .map_err(|_| "local_profile_upgrade_write_failed")?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| "local_profile_upgrade_write_failed")?;
            fs::rename(&temp, &self.path).map_err(|_| "local_profile_upgrade_write_failed")?;
            contexthub_platform_adapter::filesystem::sync_directory(
                self.path.parent().ok_or("local_profile_path_invalid")?,
            )
            .map_err(|_| "local_profile_upgrade_sync_failed")?;
        }
        let bytes = fs::read(&self.path).map_err(|_| "local_profile_read_failed")?;
        let current: PersonalProfileManifest =
            serde_json::from_slice(&bytes).map_err(|_| "local_profile_manifest_invalid")?;
        if current != self.manifest || current.schema_version != PERSONAL_DATA_VERSION {
            return Err("local_profile_changed_during_upgrade".into());
        }
        persistence
            .finish_personal_profile_upgrade(
                &self.manifest.profile_id,
                instance_fence,
                &format!("{:x}", Sha256::digest(&bytes)),
            )
            .map_err(|e| e.to_string())
    }
}
