use contexthub_platform_adapter::filesystem::{
    is_cloud_placeholder_file, is_link_or_reparse_point, path_for_comparison,
    symlink_metadata_with_stable_identity,
};
use serde_json::Value;
use std::path::{Component, Path};

pub(super) fn native_archive_state_from_path(home: &Path, thread: &Value) -> Result<bool, String> {
    let fail = || "codex_thread_archive_state_unconfirmed".to_owned();
    let path = Path::new(thread["path"].as_str().ok_or_else(fail)?);
    if !path.is_absolute() || !home.is_absolute() {
        return Err(fail());
    }
    let relative = path.strip_prefix(home).map_err(|_| fail())?;
    let mut parts = relative.components();
    let archived = match parts.next() {
        Some(Component::Normal(root)) if root == "sessions" => false,
        Some(Component::Normal(root)) if root == "archived_sessions" => true,
        _ => return Err(fail()),
    };
    if parts.clone().next().is_none() || !parts.all(|part| matches!(part, Component::Normal(_))) {
        return Err(fail());
    }
    let mut identities = Vec::new();
    for parent in path.ancestors() {
        let metadata = symlink_metadata_with_stable_identity(parent).map_err(|_| fail())?;
        if is_link_or_reparse_point(&metadata.metadata)
            || is_cloud_placeholder_file(&metadata.metadata)
            || (parent == path && !metadata.metadata.is_file())
            || (parent != path && !metadata.metadata.is_dir())
        {
            return Err(fail());
        }
        identities.push((parent, metadata.stable_identity));
    }
    let canonical_home = home.canonicalize().map_err(|_| fail())?;
    let canonical_path = path.canonicalize().map_err(|_| fail())?;
    if path_for_comparison(&canonical_home) != path_for_comparison(home)
        || path_for_comparison(&canonical_path) != path_for_comparison(path)
        || !canonical_path.starts_with(&canonical_home)
    {
        return Err(fail());
    }
    for (path, expected) in identities {
        let current = symlink_metadata_with_stable_identity(path).map_err(|_| fail())?;
        if current.stable_identity != expected || is_link_or_reparse_point(&current.metadata) {
            return Err(fail());
        }
    }
    Ok(archived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn only_real_in_scope_rollout_paths_can_prove_archive_state() {
        let owner = crate::ai_runtime::model_test_fixture::fixture_root("native-archive-path")
            .canonicalize()
            .unwrap();
        let home = owner.join("scope-one");
        let current = home.join("sessions/2026/10/rollout.jsonl");
        let archived = home.join("archived_sessions/rollout.jsonl");
        for path in [&current, &archived] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "isolated path metadata only").unwrap();
        }
        assert!(!native_archive_state_from_path(&home, &json!({"path":current})).unwrap());
        assert!(native_archive_state_from_path(&home, &json!({"path":archived})).unwrap());
        for path in [
            owner.join("scope-two/sessions/rollout.jsonl"),
            home.join("sessions/../config.toml"),
            home.join("unrecognized/rollout.jsonl"),
        ] {
            assert!(native_archive_state_from_path(&home, &json!({"path":path})).is_err());
        }
        assert!(native_archive_state_from_path(&home, &json!({"path":null})).is_err());
        fs::remove_dir_all(owner).unwrap();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn rollout_parent_symlinks_are_rejected_even_when_the_target_stays_in_the_same_scope() {
        let owner = crate::ai_runtime::model_test_fixture::fixture_root("native-archive-symlink")
            .canonicalize()
            .unwrap();
        let home = owner.join("scope");
        let source = home.join("real");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("rollout.jsonl"), "isolated metadata").unwrap();
        std::os::unix::fs::symlink(&source, home.join("sessions")).unwrap();
        assert!(
            native_archive_state_from_path(
                &home,
                &json!({"path":home.join("sessions/rollout.jsonl")})
            )
            .is_err()
        );
        fs::remove_dir_all(owner).unwrap();
    }
}
