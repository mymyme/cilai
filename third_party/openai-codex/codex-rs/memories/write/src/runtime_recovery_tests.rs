use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn validated_git_upgrade_recovers_exhausted_job_once_and_keeps_previous_error()
-> anyhow::Result<()> {
    for (error, recoverable) in [
        (
            "记忆整理失败：执行超时、超出预算或产物校验失败，请查看本次整理日志",
            true,
        ),
        ("记忆整理失败：write git baseline index from HEAD", true),
        ("记忆整理失败：MEMORY-failed_empty_publication", true),
        (
            "MEMORY-failed_workspace_status: read MEMORY.md: 拒绝访问。 (os error 5)",
            true,
        ),
        ("MEMORY-failed_workspace_status: invalid data", false),
        (
            "MEMORY-failed_invalid_artifacts: read memory summary artifact: os error 2",
            true,
        ),
        ("MEMORY-failed_invalid_artifacts: invalid index", false),
        ("upstream timeout", false),
    ] {
        let home = Arc::new(TempDir::new()?);
        tokio::fs::create_dir(home.path().join("memory-publication")).await?;
        crate::publication::status(home.path(), "failed", Some(error)).await?;
        let db = init_state_db(&home).await?;
        let Phase2JobClaimOutcome::Claimed {
            ownership_token, ..
        } = db
            .memories()
            .try_claim_global_phase2_job(ThreadId::new(), 60)
            .await?
        else {
            panic!("initial claim")
        };
        db.memories()
            .mark_global_phase2_job_failed(&ownership_token, "failed_empty_publication", 3600)
            .await?;
        crate::retry::prepare_runtime_policy(home.path(), &db, Some(error), None).await?;
        assert_eq!(
            db.memories()
                .try_claim_global_phase2_job(ThreadId::new(), 60)
                .await?,
            Phase2JobClaimOutcome::SkippedRetryUnavailable
        );
        tokio::fs::write(
            home.path().join("memory-publication/runtime-policy.json"),
            br#"{"version":"git-baseline-recovery-v1:mingit-test-1"}"#,
        )
        .await?;
        crate::retry::prepare_runtime_policy(home.path(), &db, Some(error), Some("mingit-test-1"))
            .await?;
        assert_eq!(
            crate::publication::last_error(home.path()).await?,
            Some(error.to_owned())
        );
        let claim = db
            .memories()
            .try_claim_global_phase2_job(ThreadId::new(), 60)
            .await?;
        if recoverable {
            let Phase2JobClaimOutcome::Claimed {
                ownership_token, ..
            } = claim
            else {
                panic!("runtime repair must retry")
            };
            db.memories()
                .mark_global_phase2_job_failed(&ownership_token, "failed_empty_publication", 3600)
                .await?;
        } else {
            assert_eq!(claim, Phase2JobClaimOutcome::SkippedRetryUnavailable);
        }
        crate::retry::prepare_runtime_policy(home.path(), &db, Some(error), Some("mingit-test-1"))
            .await?;
        assert_eq!(
            db.memories()
                .try_claim_global_phase2_job(ThreadId::new(), 60)
                .await?,
            Phase2JobClaimOutcome::SkippedRetryUnavailable
        );
    }
    Ok(())
}
