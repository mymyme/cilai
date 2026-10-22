use super::native_files;

#[test]
fn native_files_stay_in_the_current_profile_and_do_not_read_workbench_publications() {
    let root = std::env::temp_dir().join(format!("cilai-native-memory-{}", uuid::Uuid::new_v4()));
    let first = root.join("profile-a");
    let second = root.join("profile-b");
    std::fs::create_dir_all(first.join("memories")).unwrap();
    std::fs::create_dir_all(first.join("memory-publication")).unwrap();
    std::fs::write(first.join("memory-publication/current"), "obsolete").unwrap();
    assert!(!native_files(&first).unwrap().has_memory);
    std::fs::write(first.join("memories/MEMORY.md"), "用户约定").unwrap();
    assert!(!native_files(&first).unwrap().has_memory);
    std::fs::write(first.join("memories/memory_summary.md"), "约定索引").unwrap();
    assert!(native_files(&first).unwrap().has_memory);
    assert!(native_files(&first).unwrap().last_updated_at.is_some());
    assert!(!native_files(&second).unwrap().has_memory);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn native_files_reject_links_outside_the_profile() {
    let root = std::env::temp_dir().join(format!("cilai-native-link-{}", uuid::Uuid::new_v4()));
    let home = root.join("profile");
    let outside = root.join("outside");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    for name in ["MEMORY.md", "memory_summary.md"] {
        std::fs::write(outside.join(name), "约定").unwrap();
    }
    std::os::unix::fs::symlink(&outside, home.join("memories")).unwrap();
    assert!(native_files(&home).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_personal_profile_requires_explicit_memory_opt_in() {
    let preferences = super::MemoryPreferences::default();
    assert!(!preferences.enabled);
    assert!(!preferences.from_tool_chats);
    let persisted: super::MemoryPreferences = serde_json::from_str("{}").unwrap();
    assert!(!persisted.enabled);
    assert!(!persisted.from_tool_chats);
}
