use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn workspace_budget_counts_sources_but_not_git_cache_or_generated_diff() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join(".git")).await?;
    fs::write(root.path().join(".git/cache"), vec![0; 500_000]).await?;
    fs::write(
        root.path().join(crate::workspace_diff::FILENAME),
        vec![0; 32_768],
    )
    .await?;
    fs::write(root.path().join("raw_memories.md"), vec![b'a'; 120_000]).await?;
    let budget = ConsolidationBudget::for_workspace(root.path()).await?;
    assert_eq!(
        (budget.input_bytes, budget.total_tokens),
        (120_000, 1_200_000)
    );
    fs::write(root.path().join("MEMORY.md"), vec![b'a'; 500_000]).await?;
    assert_eq!(
        ConsolidationBudget::for_workspace(root.path())
            .await?
            .total_tokens,
        2_400_000
    );
    fs::write(root.path().join("oversized.md"), vec![0; 16 * 1024 * 1024]).await?;
    assert!(
        ConsolidationBudget::for_workspace(root.path())
            .await
            .is_err()
    );
    Ok(())
}
