use super::*;
use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn extraction_recovery_runs_once_per_configuration_and_preserves_published_status()
-> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    fs::create_dir(home.path().join("memory-publication")).await?;
    let status = b"{\"phase\":\"failed\",\"error\":\"original failure\"}";
    fs::write(home.path().join("memory-publication/status.json"), status).await?;
    let db = codex_state::StateRuntime::init(
        codex_state::SqliteConfig::new_for_testing(home.path().abs()),
        "test-provider".to_owned(),
    )
    .await?;
    prepare_extraction_recovery(home.path(), &db, "first").await?;
    let marker = home
        .path()
        .join("memory-publication/extraction-recovery.json");
    let before = fs::metadata(&marker).await?.modified()?;
    prepare_extraction_recovery(home.path(), &db, "first").await?;
    assert_eq!(fs::metadata(&marker).await?.modified()?, before);
    assert_eq!(
        fs::read(home.path().join("memory-publication/status.json")).await?,
        status
    );
    prepare_extraction_recovery(home.path(), &db, "second").await?;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(marker).await?)?["configurationKey"],
        "second"
    );
    Ok(())
}

#[tokio::test]
async fn configuration_failure_waits_for_change_and_transient_failure_does_not_block()
-> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    fs::create_dir(home.path().join("memory-publication")).await?;
    let error = "记忆整理失败：{\"error\":{\"retryPolicy\":\"fix_model_configuration\"}}";
    record(home.path(), "one", Some(error)).await?;
    assert_eq!(blocked(home.path(), "one").await?, Some(error.to_owned()));
    assert_eq!(blocked(home.path(), "two").await?, None);
    record(home.path(), "two", None).await?;
    assert_eq!(blocked(home.path(), "one").await?, None);
    record(home.path(), "two", Some("upstream timeout")).await?;
    assert_eq!(blocked(home.path(), "two").await?, None);
    Ok(())
}

#[tokio::test]
async fn quota_failure_blocks_repeated_worker_start_until_explicit_resume_or_model_change()
-> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    fs::create_dir(home.path().join("memory-publication")).await?;
    let old_status = b"previous published memory";
    fs::write(home.path().join("memory-publication/current"), old_status).await?;
    for error in [
        codex_protocol::error::CodexErr::QuotaExceeded.to_string(),
        "记忆整理失败：{\"error\":{\"code\":\"AI-QUOTA-001\",\"retryPolicy\":\"add_quota\"}}"
            .to_owned(),
    ] {
        record(home.path(), "same", Some(&error)).await?;
        for _ in 0..3 {
            assert_eq!(blocked(home.path(), "same").await?, Some(error.clone()));
        }
        assert_eq!(blocked(home.path(), "changed").await?, None);
        fs::remove_file(home.path().join("memory-publication/quota-error.json")).await?;
        assert_eq!(blocked(home.path(), "same").await?, None);
        assert_eq!(
            fs::read(home.path().join("memory-publication/current")).await?,
            old_status
        );
    }
    Ok(())
}
