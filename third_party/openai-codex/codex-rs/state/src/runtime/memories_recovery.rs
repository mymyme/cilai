use super::*;

impl MemoryStore {
    /// Restore the attempt allowance after a memory-worker configuration change.
    /// Existing failures, watermarks and backoff remain authoritative until a job succeeds.
    pub async fn rearm_exhausted_stage1_jobs(&self) -> anyhow::Result<u64> {
        Ok(sqlx::query(
            "UPDATE jobs SET retry_remaining = ? WHERE kind = ? AND status = 'error' \
             AND retry_remaining <= 0 AND (lease_until IS NULL OR lease_until <= ?)",
        )
        .bind(DEFAULT_RETRY_REMAINING)
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(Utc::now().timestamp())
        .execute(self.pool.as_ref())
        .await?
        .rows_affected())
    }
}

impl StateRuntime {
    /// Claim previously failed extractions independently of the new-source age window.
    /// Only managed memory recovery calls this; ownership, backoff and source isolation still apply.
    pub async fn claim_failed_stage1_jobs_for_recovery(
        &self,
        worker_id: ThreadId,
        params: Stage1StartupClaimParams<'_>,
    ) -> anyhow::Result<Vec<Stage1JobClaim>> {
        if params.max_claimed == 0 || params.scan_limit == 0 {
            return Ok(Vec::new());
        }
        let now = Utc::now();
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT job_key FROM jobs WHERE kind = ? AND status = 'error' \
             AND retry_remaining > 0 AND (retry_at IS NULL OR retry_at <= ?) \
             AND (lease_until IS NULL OR lease_until <= ?) ORDER BY started_at DESC LIMIT ?",
        )
        .bind(JOB_KIND_MEMORY_STAGE1)
        .bind(now.timestamp())
        .bind(now.timestamp())
        .bind(i64::try_from(params.scan_limit).unwrap_or(i64::MAX))
        .fetch_all(self.memories.pool.as_ref())
        .await?;
        let idle_cutoff = now - Duration::hours(params.min_rollout_idle_hours.max(0));
        let mut claimed = Vec::new();
        for id in ids {
            if claimed.len() >= params.max_claimed {
                break;
            }
            let id = ThreadId::from_string(&id)?;
            if id == worker_id {
                continue;
            }
            let Some(thread) = self.get_thread(id).await? else {
                continue;
            };
            if thread.archived_at.is_some()
                || thread.updated_at > idle_cutoff
                || (!params.allowed_sources.is_empty()
                    && !params.allowed_sources.contains(&thread.source))
                || self.get_thread_memory_mode(id).await?.as_deref() != Some("enabled")
            {
                continue;
            }
            if let Stage1JobClaimOutcome::Claimed { ownership_token } = self
                .memories()
                .try_claim_stage1_job(
                    id,
                    worker_id,
                    thread.updated_at.timestamp(),
                    params.lease_seconds,
                    params.max_claimed,
                )
                .await?
            {
                claimed.push(Stage1JobClaim {
                    thread,
                    ownership_token,
                });
            }
        }
        Ok(claimed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_utils_absolute_path::test_support::PathExt;

    #[tokio::test]
    async fn recovery_claims_old_failed_sources_without_changing_times_or_isolation()
    -> anyhow::Result<()> {
        let home = crate::runtime::test_support::unique_temp_dir();
        let runtime = StateRuntime::init(
            crate::SqliteConfig::new_for_testing(home.as_path().abs()),
            "test-provider".to_owned(),
        )
        .await?;
        let worker = ThreadId::new();
        let mut eligible = None;
        for name in [
            "old", "archived", "disabled", "recent", "foreign", "cooling",
        ] {
            let id = ThreadId::new();
            let mut thread = crate::runtime::test_support::test_thread_metadata(
                &home,
                id,
                home.join("workspace"),
            );
            if name == "archived" {
                thread.archived_at = Some(thread.updated_at);
            }
            if name == "recent" {
                thread.updated_at = Utc::now();
            }
            if name == "foreign" {
                thread.source = "foreign".to_owned();
            }
            runtime.upsert_thread(&thread).await?;
            if name == "disabled" {
                sqlx::query("UPDATE threads SET memory_mode = 'disabled' WHERE id = ?")
                    .bind(id.to_string())
                    .execute(runtime.pool.as_ref())
                    .await?;
            }
            sqlx::query("INSERT INTO jobs (kind,job_key,status,retry_remaining,input_watermark,retry_at) VALUES (?,?,'error',3,?,?)")
                .bind(JOB_KIND_MEMORY_STAGE1).bind(id.to_string()).bind(thread.updated_at.timestamp())
                .bind((name == "cooling").then(|| Utc::now().timestamp() + 3600))
                .execute(runtime.memories.pool.as_ref()).await?;
            if name == "old" {
                eligible = Some(thread);
            }
        }
        let sources = vec!["cli".to_owned()];
        let params = Stage1StartupClaimParams {
            scan_limit: 100,
            max_claimed: 2,
            max_age_days: 10,
            min_rollout_idle_hours: 6,
            allowed_sources: &sources,
            lease_seconds: 3600,
        };
        assert!(
            runtime
                .memories()
                .claim_stage1_jobs_for_startup(worker, params)
                .await?
                .is_empty()
        );
        let claims = runtime
            .claim_failed_stage1_jobs_for_recovery(worker, params)
            .await?;
        assert_eq!(claims.len(), 1);
        assert_eq!(Some(claims[0].thread.clone()), eligible);
        assert!(
            runtime
                .claim_failed_stage1_jobs_for_recovery(worker, params)
                .await?
                .is_empty()
        );
        let _ = tokio::fs::remove_dir_all(home).await;
        Ok(())
    }

    #[tokio::test]
    async fn recovery_preserves_backoff_failures_success_and_active_ownership() -> anyhow::Result<()>
    {
        let home = crate::runtime::test_support::unique_temp_dir();
        let runtime = StateRuntime::init(
            crate::SqliteConfig::new_for_testing(home.as_path().abs()),
            "test-provider".to_owned(),
        )
        .await?;
        let store = runtime.memories();
        let future = Utc::now().timestamp() + 3600;
        for (key, kind, status, retries, lease) in [
            ("exhausted", JOB_KIND_MEMORY_STAGE1, "error", 0, None),
            ("pending", JOB_KIND_MEMORY_STAGE1, "error", 1, None),
            (
                "running",
                JOB_KIND_MEMORY_STAGE1,
                "running",
                0,
                Some(future),
            ),
            ("succeeded", JOB_KIND_MEMORY_STAGE1, "done", 0, None),
            (
                "global",
                JOB_KIND_MEMORY_CONSOLIDATE_GLOBAL,
                "error",
                0,
                None,
            ),
        ] {
            sqlx::query("INSERT INTO jobs (kind,job_key,status,retry_remaining,retry_at,last_error,input_watermark,last_success_watermark,lease_until) VALUES (?,?,?,?,?,'original failure',100,50,?)")
                .bind(kind).bind(key).bind(status).bind(retries).bind(future).bind(lease)
                .execute(store.pool.as_ref()).await?;
        }
        assert_eq!(store.rearm_exhausted_stage1_jobs().await?, 1);
        let rows: Vec<(String, String, i64, i64, String, i64, i64, Option<i64>)> =
            sqlx::query_as("SELECT job_key,status,retry_remaining,retry_at,last_error,input_watermark,last_success_watermark,lease_until FROM jobs ORDER BY job_key")
                .fetch_all(store.pool.as_ref()).await?;
        assert_eq!(
            rows,
            vec![
                (
                    "exhausted".to_owned(),
                    "error".to_owned(),
                    3,
                    future,
                    "original failure".to_owned(),
                    100,
                    50,
                    None
                ),
                (
                    "global".to_owned(),
                    "error".to_owned(),
                    0,
                    future,
                    "original failure".to_owned(),
                    100,
                    50,
                    None
                ),
                (
                    "pending".to_owned(),
                    "error".to_owned(),
                    1,
                    future,
                    "original failure".to_owned(),
                    100,
                    50,
                    None
                ),
                (
                    "running".to_owned(),
                    "running".to_owned(),
                    0,
                    future,
                    "original failure".to_owned(),
                    100,
                    50,
                    Some(future)
                ),
                (
                    "succeeded".to_owned(),
                    "done".to_owned(),
                    0,
                    future,
                    "original failure".to_owned(),
                    100,
                    50,
                    None
                ),
            ]
        );
        assert_eq!(store.rearm_exhausted_stage1_jobs().await?, 0);
        let _ = tokio::fs::remove_dir_all(home).await;
        Ok(())
    }
}
