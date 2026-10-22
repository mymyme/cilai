use super::*;

pub(super) async fn claim(
    context: &MemoryStartupContext,
    db: &StateRuntime,
    home: &Path,
) -> Result<Claim, String> {
    let claim = db
        .memories()
        .try_claim_global_phase2_job(context.thread_id(), crate::stage_two::JOB_LEASE_SECONDS)
        .await
        .map_err(|e| {
            tracing::error!("failed to claim job: {e}");
            format!("领取记忆整理任务失败：{e:#}")
        })?;
    let (token, watermark) = match claim {
        codex_state::Phase2JobClaimOutcome::Claimed {
            ownership_token,
            input_watermark,
        } => {
            context.counter(
                MEMORY_PHASE_TWO_JOBS,
                /*inc*/ 1,
                &[("status", "claimed")],
            );
            (ownership_token, input_watermark)
        }
        codex_state::Phase2JobClaimOutcome::SkippedRetryUnavailable => {
            return Err("skipped_retry_unavailable".to_owned());
        }
        codex_state::Phase2JobClaimOutcome::SkippedCooldown => {
            return Err("skipped_cooldown".to_owned());
        }
        codex_state::Phase2JobClaimOutcome::SkippedRunning => {
            return Err("skipped_running".to_owned());
        }
    };

    Ok(Claim {
        token,
        watermark,
        home: home.to_path_buf(),
    })
}

pub(super) async fn failed(
    context: &MemoryStartupContext,
    db: &StateRuntime,
    claim: &Claim,
    reason: &'static str,
    detail: &str,
) {
    crate::failure::record(&claim.home, reason, detail).await;
    context.counter(MEMORY_PHASE_TWO_JOBS, /*inc*/ 1, &[("status", reason)]);
    if matches!(
        db.memories()
            .mark_global_phase2_job_failed(
                &claim.token,
                reason,
                crate::stage_two::JOB_RETRY_DELAY_SECONDS,
            )
            .await,
        Ok(false)
    ) {
        let _ = db
            .memories()
            .mark_global_phase2_job_failed_if_unowned(
                &claim.token,
                reason,
                crate::stage_two::JOB_RETRY_DELAY_SECONDS,
            )
            .await;
    }
}

pub(super) async fn succeed(
    context: &MemoryStartupContext,
    db: &StateRuntime,
    claim: &Claim,
    completion_watermark: i64,
    selected_outputs: &[codex_state::Stage1Output],
    reason: &'static str,
) -> bool {
    context.counter(MEMORY_PHASE_TWO_JOBS, /*inc*/ 1, &[("status", reason)]);
    match db
        .memories()
        .mark_global_phase2_job_succeeded(&claim.token, completion_watermark, selected_outputs)
        .await
    {
        Ok(true) => true,
        result => {
            let detail = match result {
                Ok(false) => "记忆整理任务归属已变化，无法确认成功".to_owned(),
                Err(error) => format!("保存记忆任务完成状态失败：{error:#}"),
                Ok(true) => unreachable!(),
            };
            crate::failure::record(&claim.home, "failed_confirm_ownership", &detail).await;
            false
        }
    }
}
