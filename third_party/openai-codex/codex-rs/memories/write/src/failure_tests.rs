use super::*;

#[tokio::test]
async fn baseline_failure_preserves_cause_and_artifact_failure_has_its_own_title()
-> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    tokio::fs::create_dir(home.path().join("memory-publication")).await?;
    for (stage, detail, title, label) in [
        (
            "failed_empty_publication",
            "write git baseline index from HEAD: Git 运行时不可用",
            "记忆目录初始化失败",
            "清理空记忆与重建基线",
        ),
        (
            "failed_invalid_artifacts",
            "memory summary artifact does not start with v1",
            "记忆产物校验失败",
            "校验记忆索引与摘要",
        ),
    ] {
        record(home.path(), stage, detail).await;
        let error = crate::publication::last_error(home.path())
            .await?
            .expect("failure persisted");
        let payload: serde_json::Value = serde_json::from_str(&error[error.find('{').unwrap()..])?;
        assert_eq!(payload["error"]["title"], title);
        assert_eq!(payload["error"]["message"], detail);
        assert_eq!(payload["error"]["diagnostics"]["stage"], label);
    }
    Ok(())
}

#[tokio::test]
async fn provider_diagnostics_keep_their_error_code_and_request_id() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    tokio::fs::create_dir(home.path().join("memory-publication")).await?;
    let detail =
        r#"{"error":{"code":"AI-TOOL-001","message":"工具参数不兼容"},"request_id":"request-123"}"#;
    record(home.path(), "failed_agent", detail).await;
    assert_eq!(
        crate::publication::last_error(home.path()).await?,
        Some(format!("记忆整理失败：{detail}"))
    );
    Ok(())
}
