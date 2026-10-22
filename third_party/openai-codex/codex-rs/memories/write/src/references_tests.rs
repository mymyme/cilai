use super::*;

#[tokio::test]
async fn validates_existing_references_and_rejects_missing_or_escaping_sources()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("rollout_summaries")).await?;
    fs::write(root.path().join("rollout_summaries/source.md"), "evidence").await?;
    validate_rollout_references(
        root.path(),
        &[
            "- rollout_summaries/source.md (cwd=/tmp)\n[link](rollout_summaries/source.md:12)",
            "Use `rollout_summaries/*.md` for details; `rollout_summaries/` is a directory.",
        ],
    )
    .await?;
    for reference in [
        "rollout_summaries/missing.md",
        "rollout_summaries/../outside.md",
        "rollout_summaries/source.md/../../outside.md",
    ] {
        assert!(
            validate_rollout_references(root.path(), &[reference])
                .await
                .is_err()
        );
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlinked_reference() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("rollout_summaries")).await?;
    fs::write(root.path().join("outside.md"), "evidence").await?;
    std::os::unix::fs::symlink(
        root.path().join("outside.md"),
        root.path().join("rollout_summaries/source.md"),
    )?;
    assert!(
        validate_rollout_references(root.path(), &["rollout_summaries/source.md"])
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn validates_native_bare_filenames_in_source_sections() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("rollout_summaries")).await?;
    fs::write(root.path().join("rollout_summaries/source.md"), "evidence").await?;
    validate_rollout_references(root.path(), &[
        "### rollout_summary_files\n- source.md (cwd=/tmp)\n- `source.md` (thread_id=example)\n### keywords\n- unrelated.md",
    ]).await?;
    for reference in [
        "missing.md",
        "../source.md",
        "not a source",
        "nested/source.md",
    ] {
        assert!(
            validate_rollout_references(
                root.path(),
                &[&format!("### rollout_summary_files\n- {reference}")]
            )
            .await
            .is_err()
        );
    }
    Ok(())
}
