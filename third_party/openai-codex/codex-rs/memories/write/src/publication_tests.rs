use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn failed_draft_never_replaces_published_memory_and_empty_input_unpublishes()
-> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let root = home.path().join("memories");
    let publication = home.path().join("memory-publication");
    fs::create_dir_all(&root).await?;
    fs::create_dir_all(&publication).await?;
    fs::write(root.join("MEMORY.md"), "用户喜欢先写结论").await?;
    fs::write(root.join("memory_summary.md"), "v1\n先写结论").await?;
    publish(&root, 1).await?;
    let revision = fs::read_to_string(publication.join("current")).await?;
    fs::write(root.join("MEMORY.md"), "").await?;
    assert!(publish(&root, 1).await.is_err());
    assert_eq!(
        fs::read_to_string(publication.join("current")).await?,
        revision
    );
    assert_eq!(
        fs::read_to_string(
            publication
                .join("versions")
                .join(revision)
                .join("memories/MEMORY.md")
        )
        .await?,
        "用户喜欢先写结论"
    );
    publish(&root, 0).await?;
    assert_eq!(fs::read_to_string(publication.join("current")).await?, "");
    Ok(())
}

#[tokio::test]
async fn extension_instructions_alone_are_not_memory_sources() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let notes = root.path().join("extensions/ad_hoc/notes");
    fs::create_dir_all(&notes).await?;
    fs::write(
        notes.parent().unwrap().join("instructions.md"),
        "instructions",
    )
    .await?;
    assert_eq!(extension_sources(root.path()).await?, 0);
    fs::write(notes.join("preference.md"), "先写结论").await?;
    assert_eq!(extension_sources(root.path()).await?, 1);
    Ok(())
}

#[tokio::test]
async fn published_resources_are_immutable_and_resource_changes_publish_a_new_version()
-> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let root = home.path().join("memories");
    let publication = home.path().join("memory-publication");
    fs::create_dir_all(root.join("rollout_summaries")).await?;
    fs::create_dir_all(&publication).await?;
    fs::write(
        root.join("MEMORY.md"),
        format!("See {}/rollout_summaries/note.md", root.display()),
    )
    .await?;
    fs::write(root.join("memory_summary.md"), "v1\n用户偏好").await?;
    fs::write(root.join("rollout_summaries/note.md"), "先写结论").await?;
    publish(&root, 1).await?;
    let revision = fs::read_to_string(publication.join("current")).await?;
    let snapshot = publication
        .join("versions")
        .join(&revision)
        .join("memories");
    assert!(
        fs::read_to_string(snapshot.join("MEMORY.md"))
            .await?
            .contains(&snapshot.display().to_string())
    );
    publish(&root, 1).await?;
    assert_eq!(
        fs::read_to_string(publication.join("current")).await?,
        revision
    );
    fs::write(root.join("rollout_summaries/note.md"), "先写结论，再列证据").await?;
    publish(&root, 1).await?;
    assert_ne!(
        fs::read_to_string(publication.join("current")).await?,
        revision
    );
    assert_eq!(
        fs::read_to_string(snapshot.join("rollout_summaries/note.md")).await?,
        "先写结论"
    );
    Ok(())
}

#[tokio::test]
async fn oversized_publication_removes_partial_snapshot_and_keeps_current_version()
-> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let root = home.path().join("memories");
    let publication = home.path().join("memory-publication");
    fs::create_dir_all(&root).await?;
    fs::create_dir_all(&publication).await?;
    fs::write(root.join("MEMORY.md"), "用户偏好").await?;
    fs::write(root.join("memory_summary.md"), "v1\n用户偏好").await?;
    publish(&root, 1).await?;
    let revision = fs::read_to_string(publication.join("current")).await?;
    fs::write(root.join("too-large.bin"), vec![0; 16 * 1024 * 1024]).await?;
    assert!(publish(&root, 1).await.is_err());
    assert_eq!(
        fs::read_to_string(publication.join("current")).await?,
        revision
    );
    let mut versions = fs::read_dir(publication.join("versions")).await?;
    assert_eq!(
        versions
            .next_entry()
            .await?
            .unwrap()
            .file_name()
            .to_string_lossy(),
        revision
    );
    assert!(versions.next_entry().await?.is_none());
    Ok(())
}

#[tokio::test]
async fn missing_rollout_reference_never_replaces_published_version() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let root = home.path().join("memories");
    let directory = home.path().join("memory-publication");
    fs::create_dir_all(root.join("rollout_summaries")).await?;
    fs::create_dir_all(&directory).await?;
    fs::write(
        root.join("rollout_summaries/evidence.md"),
        "verified preference",
    )
    .await?;
    fs::write(root.join("MEMORY.md"), "See rollout_summaries/evidence.md").await?;
    fs::write(root.join("memory_summary.md"), "v1\nExisting preference").await?;
    publish(&root, 1).await?;
    let revision = fs::read_to_string(directory.join("current")).await?;
    fs::write(root.join("MEMORY.md"), "See rollout_summaries/missing.md").await?;
    assert!(publish(&root, 1).await.is_err());
    assert_eq!(
        fs::read_to_string(directory.join("current")).await?,
        revision
    );
    Ok(())
}
