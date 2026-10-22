use std::path::{Path, PathBuf};

pub fn business_directory(owner: &Path, environment: &str) -> Result<PathBuf, String> {
    if !matches!(environment, "dev" | "prod") {
        return Err("宿主自检数据环境无效".into());
    }
    let directory = owner
        .join(environment)
        .join(crate::personal_profile::APPLICATION_ID)
        .join("business");
    crate::personal_profile::validate_data_directory(&directory)?;
    Ok(directory)
}

pub fn create(arguments: &[String], temporary: &Path) -> Result<Option<PathBuf>, String> {
    if arguments.is_empty() {
        return Ok(None);
    }
    if arguments.len() != 3 || arguments[0] != "--headless-check" || arguments[1] != "--data-dir" {
        return Err("宿主自检数据参数无效".into());
    }
    let requested = PathBuf::from(&arguments[2]);
    if !requested.is_absolute() || requested.file_name() != Some(std::ffi::OsStr::new("host")) {
        return Err("宿主自检数据目录无效".into());
    }
    let temporary = temporary
        .canonicalize()
        .map_err(|error| format!("无法定位自检临时目录：{error}"))?;
    let parent = requested
        .parent()
        .ok_or("宿主自检数据目录无效")?
        .canonicalize()
        .map_err(|error| format!("无法定位自检会话目录：{error}"))?;
    if parent.parent() != Some(temporary.as_path())
        || !parent
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("cilai-headless-"))
    {
        return Err("宿主自检数据目录必须位于独立临时会话中".into());
    }
    let directory = parent.join("host");
    std::fs::create_dir(&directory)
        .map_err(|error| format!("无法新建宿主自检数据目录，不允许复用既有资料：{error}"))?;
    Ok(Some(directory))
}

#[cfg(test)]
mod tests {
    use super::{business_directory, create};

    #[test]
    fn self_check_uses_the_personal_profile_namespace() {
        let owner = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("cilai-headless-layout-{}", uuid::Uuid::new_v4()));
        for environment in ["dev", "prod"] {
            let directory = business_directory(&owner, environment).unwrap();
            let ordinary = owner
                .join(environment)
                .join(crate::personal_profile::APPLICATION_ID)
                .join("business");
            assert_eq!(directory, ordinary);
            crate::personal_profile::validate_data_directory(&directory).unwrap();
            let profile = crate::personal_profile::load_or_create(&directory).unwrap();
            assert_eq!(
                profile.directory.parent().unwrap().parent().unwrap(),
                directory
            );
        }
        for environment in ["", "../prod", "other"] {
            assert!(business_directory(&owner, environment).is_err());
        }
        std::fs::remove_dir_all(owner).unwrap();
    }

    #[test]
    fn self_check_cannot_adopt_existing_or_employee_data() {
        let temporary = std::env::temp_dir().join(format!(
            "cilai-host-isolation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let session = temporary.join("cilai-headless-fresh");
        std::fs::create_dir_all(&session).unwrap();
        let host = session.join("host");
        let arguments = vec![
            "--headless-check".into(),
            "--data-dir".into(),
            host.display().to_string(),
        ];
        assert!(create(&[], &temporary).unwrap().is_none());
        assert_eq!(
            create(&arguments, &temporary).unwrap().unwrap(),
            host.canonicalize().unwrap()
        );
        assert!(
            create(&arguments, &temporary)
                .unwrap_err()
                .contains("不允许复用")
        );
        let employee = temporary.join("employee");
        std::fs::create_dir(&employee).unwrap();
        std::fs::write(employee.join("credentials.json"), "employee").unwrap();
        for path in [
            employee.clone(),
            employee.join("host"),
            std::path::PathBuf::from("relative/host"),
        ] {
            let arguments = vec![
                "--headless-check".into(),
                "--data-dir".into(),
                path.display().to_string(),
            ];
            assert!(create(&arguments, &temporary).is_err());
        }
        assert!(
            create(
                &["--data-dir".into(), employee.display().to_string()],
                &temporary
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(employee.join("credentials.json")).unwrap(),
            "employee"
        );
        std::fs::remove_dir_all(temporary).unwrap();
    }
}
