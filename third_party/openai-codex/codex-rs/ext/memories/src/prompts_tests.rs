use super::*;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;
use tempfile::tempdir;
use tokio::fs as tokio_fs;

#[tokio::test]
async fn build_memory_tool_developer_instructions_renders_embedded_template() {
    let temp = tempdir().unwrap();
    let codex_home = AbsolutePathBuf::from_absolute_path(temp.path()).unwrap();
    let memories_dir = codex_home.join("memories");
    tokio_fs::create_dir_all(&memories_dir).await.unwrap();
    tokio_fs::write(
        memories_dir.join("memory_summary.md"),
        "Short memory summary for tests.",
    )
    .await
    .unwrap();

    let instructions = build_memory_tool_developer_instructions(&codex_home)
        .await
        .unwrap();

    assert!(instructions.contains(&format!(
        "- {}/memory_summary.md (already provided below; do NOT open again)",
        memories_dir.display()
    )));
    assert!(instructions.contains("Short memory summary for tests."));
    assert_eq!(
        instructions
            .matches("========= MEMORY_SUMMARY BEGINS =========")
            .count(),
        1
    );
}

#[tokio::test]
async fn managed_memory_injects_only_the_published_revision() {
    let temp = tempdir().unwrap();
    let home = AbsolutePathBuf::from_absolute_path(temp.path()).unwrap();
    let working = home.join("memories");
    tokio_fs::create_dir_all(&working).await.unwrap();
    tokio_fs::write(
        working.join("memory_summary.md"),
        "uncommitted private draft",
    )
    .await
    .unwrap();
    let publication = home.join("memory-publication");
    tokio_fs::create_dir_all(&publication).await.unwrap();
    assert!(
        build_memory_tool_developer_instructions(&home)
            .await
            .is_none()
    );
    let revision = "00000000-0000-4000-8000-000000000001";
    let published = publication.join("versions").join(revision).join("memories");
    tokio_fs::create_dir_all(&published).await.unwrap();
    tokio_fs::write(
        published.join("memory_summary.md"),
        "v1\npublished preference",
    )
    .await
    .unwrap();
    tokio_fs::write(publication.join("current"), revision)
        .await
        .unwrap();
    let instructions = build_memory_tool_developer_instructions(&home)
        .await
        .unwrap();
    assert!(instructions.contains("published preference"));
    assert!(!instructions.contains("uncommitted private draft"));
    assert!(instructions.contains(&published.display().to_string()));
    assert!(instructions.contains(&format!("{}/extensions/ad_hoc/notes/", working.display())));
}
