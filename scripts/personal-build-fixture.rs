use std::path::{Component, Path, PathBuf};

pub(crate) fn personal_build_root_for(
    manifest: &Path,
    home: &Path,
    registration: &serde_json::Value,
) -> Result<PathBuf, &'static str> {
    let clean = |path: &Path| {
        path.is_absolute()
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    };
    if !clean(manifest) || !clean(home) {
        return Err("personal_test_layout_invalid");
    }
    let expected = home.join(".cilai-personal-builds/desktop");
    if home.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            name.as_str(),
            "mobile documents"
                | "cloudstorage"
                | "dropbox"
                | "google drive"
                | "googledrive"
                | "icloud drive"
        ) || name == "onedrive"
            || name.starts_with("onedrive - ")
    }) {
        return Err("personal_test_build_root_cloud_synced");
    }
    let source = registration["sourceRoot"]
        .as_str()
        .map(Path::new)
        .ok_or("personal_test_registration_invalid")?;
    let project = registration["projectRoot"]
        .as_str()
        .map(Path::new)
        .ok_or("personal_test_registration_invalid")?;
    if registration["schemaVersion"] != 1
        || registration["applicationId"] != "cn.cilai.personal.workbench"
        || registration["buildRoot"].as_str().map(Path::new) != Some(expected.as_path())
        || !clean(source)
        || !clean(project)
        || source != project.join("06代码索引/cilai-workbench")
    {
        return Err("personal_test_registration_invalid");
    }
    let suffixes = [
        "crates/platform-adapter",
        "crates/workbench-runtime",
        "apps/desktop/host-service",
    ];
    if !suffixes.iter().any(|suffix| {
        [
            source.to_path_buf(),
            expected.clone(),
            expected.join("runtime-source"),
        ]
        .into_iter()
        .any(|base| manifest == base.join(suffix))
    }) {
        return Err("personal_test_layout_invalid");
    }
    Ok(expected)
}

pub(crate) fn personal_build_root() -> PathBuf {
    let home = std::env::home_dir().expect("测试需要明确的用户主目录");
    let root = home.join(".cilai-personal-builds/desktop");
    let file = root.join(".cilai-personal-build-registration.json");
    for path in file.ancestors() {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => assert!(
                !metadata.file_type().is_symlink(),
                "个人测试构建根不能经过链接"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("个人测试构建根无法核对：{error}"),
        }
    }
    let registration: serde_json::Value = serde_json::from_slice(
        &std::fs::read(file).expect("个人测试需要已登记的本地构建根，请先暂存源码"),
    )
    .expect("个人测试构建根登记无效");
    personal_build_root_for(Path::new(env!("CARGO_MANIFEST_DIR")), &home, &registration)
        .expect("个人测试必须使用登记源码或个人本地构建根")
}

#[cfg(test)]
mod personal_build_location_tests {
    use super::*;

    #[test]
    fn source_and_staged_fixtures_share_registered_local_root() {
        let home = std::env::temp_dir().join("personal-fixture-layout-home");
        let project = std::env::temp_dir().join("personal-fixture-layout-project");
        let source = project.join("06代码索引/cilai-workbench");
        let expected = home.join(".cilai-personal-builds/desktop");
        let registration = serde_json::json!({
            "schemaVersion": 1, "applicationId": "cn.cilai.personal.workbench",
            "buildRoot": expected, "sourceRoot": source, "projectRoot": project,
        });
        for base in [&source, &expected, &expected.join("runtime-source")] {
            for suffix in [
                "crates/platform-adapter",
                "crates/workbench-runtime",
                "apps/desktop/host-service",
            ] {
                assert_eq!(
                    personal_build_root_for(&base.join(suffix), &home, &registration).unwrap(),
                    expected
                );
            }
        }
        for manifest in [
            PathBuf::from("relative/crates/platform-adapter"),
            project.join("unknown/crates/platform-adapter"),
            project.join(".personal-build/crates/platform-adapter"),
            expected.join("../unknown/crates/platform-adapter"),
        ] {
            assert!(personal_build_root_for(&manifest, &home, &registration).is_err());
        }
        let mut altered = registration.clone();
        altered["sourceRoot"] = serde_json::json!(project.join("wrong"));
        assert!(
            personal_build_root_for(&source.join("crates/platform-adapter"), &home, &altered)
                .is_err()
        );
        assert!(
            personal_build_root_for(
                &source.join("crates/platform-adapter"),
                &home.join("OneDrive"),
                &registration
            )
            .is_err()
        );
    }
}
