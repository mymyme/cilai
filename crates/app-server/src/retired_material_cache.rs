use crate::AppServerServices;
use std::{fs, io, path::Path};

impl AppServerServices {
    /// 在独占当前profile的实例启动阶段调用。旧任务仍可能持有文件时保留缓存，下一次启动重试。
    /// # Errors
    /// 无法确认读取者状态、缓存边界或删除失败时返回错误，不删除其他路径。
    pub fn retire_material_cache(&mut self) -> io::Result<()> {
        let Some(profile) = self.data_dir.clone() else {
            return Ok(());
        };
        let cache = profile.join("cloud-materials");
        if !cache.try_exists()? {
            return Ok(());
        }
        let pids = self
            .persistence
            .as_ref()
            .ok_or_else(|| io::Error::other("retired_material_cache_state_unavailable"))?
            .ai_material_reader_pids()
            .map_err(|_| io::Error::other("retired_material_cache_state_unavailable"))?;
        for pid in pids {
            if contexthub_platform_adapter::instance::process_start_fingerprint(pid)?.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "retired_material_cache_reader_active",
                ));
            }
        }
        clean(&profile)
    }
}
fn directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::other("retired_material_cache_scope_invalid"));
    }
    Ok(())
}
fn clean(profile: &Path) -> io::Result<()> {
    let profile = profile.canonicalize()?;
    let cache = profile.join("cloud-materials");
    directory(&cache)?;
    for node in fs::read_dir(&cache)? {
        let node = node?;
        directory(&node.path())?;
        uuid::Uuid::parse_str(&node.file_name().to_string_lossy())
            .map_err(|_| io::Error::other("retired_material_cache_layout_invalid"))?;
        for revision in fs::read_dir(node.path())? {
            let revision = revision?;
            directory(&revision.path())?;
            revision
                .file_name()
                .to_string_lossy()
                .parse::<u64>()
                .map_err(|_| io::Error::other("retired_material_cache_layout_invalid"))?;
            for file in fs::read_dir(revision.path())? {
                let file = file?;
                if !file.file_type()?.is_file() {
                    return Err(io::Error::other("retired_material_cache_scope_invalid"));
                }
            }
        }
    }
    fs::remove_dir_all(cache)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_only_managed_cloud_materials_and_preserves_downloads() {
        let profile =
            std::env::temp_dir().join(format!("cilai-cache-retirement-{}", uuid::Uuid::new_v4()));
        let cache = profile
            .join("cloud-materials")
            .join(uuid::Uuid::new_v4().to_string())
            .join("1");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("file.docx"), b"cache").unwrap();
        fs::create_dir_all(profile.join("downloads")).unwrap();
        fs::write(profile.join("downloads/user.docx"), b"user").unwrap();
        clean(&profile).unwrap();
        assert!(!profile.join("cloud-materials").exists());
        assert_eq!(
            fs::read(profile.join("downloads/user.docx")).unwrap(),
            b"user"
        );
        fs::remove_dir_all(profile).unwrap();
    }
    #[test]
    fn unexpected_cache_layout_is_preserved_for_inspection() {
        let profile =
            std::env::temp_dir().join(format!("cilai-cache-retirement-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(profile.join("cloud-materials/unexpected")).unwrap();
        assert!(clean(&profile).is_err());
        assert!(profile.join("cloud-materials/unexpected").exists());
        fs::remove_dir_all(profile).unwrap();
    }
}
