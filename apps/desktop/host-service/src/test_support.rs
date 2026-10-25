use std::path::{Component, Path, PathBuf};

mod build_location {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/personal-build-fixture.rs"
    ));
}

pub(crate) fn fixture_root(scope: &str) -> PathBuf {
    let scope = Path::new(scope);
    assert!(
        !scope.as_os_str().is_empty()
            && scope
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
    );
    let root = build_location::personal_build_root()
        .join(scope)
        .join(uuid::Uuid::new_v4().simple().to_string());
    for ancestor in root.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => assert!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "宿主测试目录不能经过链接或普通文件"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("宿主测试目录无法核对：{error}"),
        }
    }
    contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
    root.canonicalize().unwrap()
}

#[cfg(test)]
mod tests {
    use super::build_location::personal_build_root_for;

    fn layout() -> (std::path::PathBuf, std::path::PathBuf, serde_json::Value) {
        let home = std::env::temp_dir().join("personal-host-home");
        let project = std::env::temp_dir().join("personal-host-layout");
        let expected = home.join(".cilai-personal-builds/desktop");
        let registration = serde_json::json!({"schemaVersion": 1, "applicationId": "cn.cilai.personal.workbench", "buildRoot": expected, "sourceRoot": project.join("06代码索引/cilai-workbench"), "projectRoot": project});
        (home, project, registration)
    }

    #[test]
    fn source_and_staged_host_tests_share_the_personal_build_root() {
        let (home, project, registration) = layout();
        let expected = home.join(".cilai-personal-builds/desktop");
        for source in [project.join("06代码索引/cilai-workbench"), expected.clone()] {
            assert_eq!(
                personal_build_root_for(
                    &source.join("apps/desktop/host-service"),
                    &home,
                    &registration
                )
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn unknown_and_traversal_layouts_do_not_choose_a_parent_directory() {
        let (home, project, registration) = layout();
        for source in [
            project.join("unknown"),
            project.join(".personal-build/../unknown"),
        ] {
            assert!(
                personal_build_root_for(
                    &source.join("apps/desktop/host-service"),
                    &home,
                    &registration
                )
                .is_err()
            );
        }
        assert!(
            personal_build_root_for(
                std::path::Path::new("apps/desktop/host-service"),
                &home,
                &registration
            )
            .is_err()
        );
    }
}
