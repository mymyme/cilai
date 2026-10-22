use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn preflight_describes_partial_recovery_without_exposing_source_content() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("MEMORY.md"), "既有索引\n").await?;
    fs::write(root.path().join("raw_memories.md"), "原始偏好\n第二行\n").await?;
    let description = describe(root.path()).await?;
    let json_start = description.find('[').unwrap();
    let json_end = description[json_start..].find("]. Missing").unwrap() + json_start + 1;
    let value: serde_json::Value = serde_json::from_str(&description[json_start..json_end])?;
    assert_eq!(
        value,
        json!([
            {"path":"MEMORY.md","exists":true,"bytes":13,"lines":1,"summaryVersionValid":null},
            {"path":"memory_summary.md","exists":false,"bytes":null,"lines":null,"summaryVersionValid":false},
            {"path":"raw_memories.md","exists":true,"bytes":23,"lines":2,"summaryVersionValid":null},
        ])
    );
    assert!(!description.contains("既有索引"));
    assert!(!description.contains("原始偏好"));
    fs::write(root.path().join("memory_summary.md"), "v1\nready").await?;
    assert!(
        describe(root.path())
            .await?
            .contains("\"summaryVersionValid\":true")
    );
    fs::write(root.path().join("memory_summary.md"), [0xff]).await?;
    assert!(describe(root.path()).await.is_err());
    Ok(())
}
