//! Authorized one-time read-only import. No legacy migrations or writes run here.
use super::{PersistenceError, SqlitePersistence, database_error};
use crate::ai_business_ledger::{digest, reject};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::path::Path;

impl SqlitePersistence {
    pub(super) fn read_legacy_memory_metadata(&self) -> Result<Value, PersistenceError> {
        let initialized: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ai_memory_state WHERE state_key='account')",
                [],
                |r| r.get(0),
            )
            .map_err(database_error("check memory migration eligibility"))?;
        if initialized {
            return Err(reject("ai_memory_legacy_already_initialized"));
        }
        let profile = self
            .database_path
            .parent()
            .ok_or_else(|| reject("ai_memory_legacy_path_invalid"))?;
        let root = profile.join("ai-runtime/codex-harness");
        let path = root.join("memories_1.sqlite");
        if !path
            .try_exists()
            .map_err(|_| reject("ai_memory_legacy_path_invalid"))?
        {
            let value = json!({"schemaVersion":1,"jobs":[],"outputs":[]});
            return Ok(json!({"digest":digest(&value),"snapshot":value}));
        }
        let profile = profile
            .canonicalize()
            .map_err(|_| reject("ai_memory_legacy_path_invalid"))?;
        for item in [&root, &path] {
            if !item
                .canonicalize()
                .map_err(|_| reject("ai_memory_legacy_path_invalid"))?
                .starts_with(&profile)
                || std::fs::symlink_metadata(item)
                    .map_err(|_| reject("ai_memory_legacy_path_invalid"))?
                    .file_type()
                    .is_symlink()
            {
                return Err(reject("ai_memory_legacy_path_invalid"));
            }
        }
        if !path.is_file() {
            return Err(reject("ai_memory_legacy_path_invalid"));
        }
        snapshot(&path)
    }
}

fn snapshot(path: &Path) -> Result<Value, PersistenceError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(database_error("open legacy memory metadata read-only"))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(database_error("bound legacy memory read"))?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(database_error("snapshot legacy memory metadata"))?;
    let unbounded: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE length(kind)>64 OR length(job_key)>256 OR length(worker_id)>256 OR length(ownership_token)>256 OR length(last_error)>65536) OR EXISTS(SELECT 1 FROM stage1_outputs WHERE length(thread_id)>256)",[],|row|row.get(0))
        .map_err(database_error("validate legacy metadata bounds"))?;
    if unbounded {
        return Err(reject("ai_memory_legacy_metadata_too_large"));
    }
    let mut statement = transaction.prepare("SELECT kind,job_key,status,worker_id,ownership_token,started_at,finished_at,lease_until,retry_at,retry_remaining,last_error,input_watermark,last_success_watermark FROM jobs ORDER BY kind,job_key LIMIT 10001")
        .map_err(database_error("read legacy memory jobs"))?;
    let mut budget = 8 * 1024 * 1024;
    let jobs = bounded(statement.query_map([], |row| Ok(json!({
        "kind":row.get::<_,String>(0)?,"key":row.get::<_,String>(1)?,"status":row.get::<_,String>(2)?,
        "workerId":row.get::<_,Option<String>>(3)?,"ownershipToken":row.get::<_,Option<String>>(4)?,
        "startedAt":row.get::<_,Option<i64>>(5)?,"finishedAt":row.get::<_,Option<i64>>(6)?,
        "leaseUntil":row.get::<_,Option<i64>>(7)?,"retryAt":row.get::<_,Option<i64>>(8)?,
        "retryRemaining":row.get::<_,i64>(9)?,"lastError":row.get::<_,Option<String>>(10)?,
        "inputWatermark":row.get::<_,Option<i64>>(11)?,"lastSuccessWatermark":row.get::<_,Option<i64>>(12)?
    }))).map_err(database_error("decode legacy memory jobs"))?, &mut budget)?;
    drop(statement);
    let mut statement = transaction.prepare("SELECT thread_id,source_updated_at,generated_at,usage_count,last_usage,selected_for_phase2,selected_for_phase2_source_updated_at FROM stage1_outputs ORDER BY thread_id LIMIT 10001")
        .map_err(database_error("read legacy memory output metadata"))?;
    let outputs = bounded(statement.query_map([], |row| Ok(json!({
        "threadId":row.get::<_,String>(0)?,"sourceUpdatedAt":row.get::<_,i64>(1)?,"generatedAt":row.get::<_,i64>(2)?,
        "usageCount":row.get::<_,Option<i64>>(3)?,"lastUsage":row.get::<_,Option<i64>>(4)?,
        "selectedForPhase2":row.get::<_,bool>(5)?,"selectedSourceUpdatedAt":row.get::<_,Option<i64>>(6)?
    }))).map_err(database_error("decode legacy memory output metadata"))?, &mut budget)?;
    drop(statement);
    transaction
        .commit()
        .map_err(database_error("finish legacy memory snapshot"))?;
    if jobs.len() > 10_000
        || outputs.len() > 10_000
        || jobs.iter().any(|job| {
            !matches!(
                job["kind"].as_str(),
                Some("memory_stage1" | "memory_consolidate_global")
            ) || !matches!(
                job["status"].as_str(),
                Some("pending" | "running" | "done" | "error" | "failed")
            ) || job["retryRemaining"]
                .as_i64()
                .is_none_or(|count| !(0..=3).contains(&count))
        })
    {
        return Err(reject("ai_memory_legacy_metadata_invalid"));
    }
    if !outputs.is_empty()
        && (outputs
            .iter()
            .any(|output| output["selectedForPhase2"] != true)
            || !jobs
                .iter()
                .any(|job| job["kind"] == "memory_consolidate_global" && job["status"] == "done"))
    {
        // Metadata alone cannot recover an unpublished extraction. Keep originals
        // and reject activation instead of silently dropping that pending work.
        return Err(reject(
            "ai_memory_legacy_unpublished_output_requires_migration",
        ));
    }
    let value = json!({"schemaVersion":1,"jobs":jobs,"outputs":outputs});
    if value.to_string().len() > 8 * 1024 * 1024 {
        return Err(reject("ai_memory_legacy_metadata_too_large"));
    }
    Ok(json!({"digest":digest(&value),"snapshot":value}))
}

fn bounded(
    rows: impl Iterator<Item = rusqlite::Result<Value>>,
    budget: &mut usize,
) -> Result<Vec<Value>, PersistenceError> {
    let mut values = Vec::new();
    for row in rows {
        let row = row.map_err(database_error("decode legacy memory metadata"))?;
        *budget = budget
            .checked_sub(row.to_string().len())
            .ok_or_else(|| reject("ai_memory_legacy_metadata_too_large"))?;
        if values.len() == 10_000 {
            return Err(reject("ai_memory_legacy_metadata_too_large"));
        }
        values.push(row);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_snapshot_preserves_lease_retry_watermarks_and_database_bytes() {
        let root = std::env::temp_dir().join(format!(
            "cilai-memory-readonly-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("memories_1.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE jobs(kind,job_key,status,worker_id,ownership_token,started_at,finished_at,lease_until,retry_at,retry_remaining,last_error,input_watermark,last_success_watermark); CREATE TABLE stage1_outputs(thread_id,source_updated_at,generated_at,usage_count,last_usage,selected_for_phase2,selected_for_phase2_source_updated_at); INSERT INTO jobs VALUES('memory_stage1','t','running','w','lease',10,NULL,30,40,1,'timeout',50,20);").unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        let result = snapshot(&path).unwrap();
        let job = &result["snapshot"]["jobs"][0];
        assert_eq!(job["retryRemaining"], 1);
        assert_eq!(job["leaseUntil"], 30);
        assert_eq!(job["lastSuccessWatermark"], 20);
        assert_eq!(before, std::fs::read(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("INSERT INTO stage1_outputs VALUES('t',50,60,0,NULL,0,NULL)")
            .unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(
            snapshot(&path).is_err(),
            "unselected output cannot be recovered from metadata alone"
        );
        assert_eq!(before, std::fs::read(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("UPDATE stage1_outputs SET selected_for_phase2=1")
            .unwrap();
        drop(db);
        assert!(
            snapshot(&path).is_err(),
            "selection does not prove consolidation completed"
        );
        let db = Connection::open(&path).unwrap();
        db.execute_batch("INSERT INTO jobs VALUES('memory_consolidate_global','global','done',NULL,NULL,50,60,NULL,NULL,3,NULL,50,50); UPDATE stage1_outputs SET usage_count=NULL;").unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        let imported = snapshot(&path).unwrap();
        assert!(imported["snapshot"]["outputs"][0]["usageCount"].is_null());
        assert!(imported["snapshot"]["outputs"][0]["lastUsage"].is_null());
        assert_eq!(before, std::fs::read(&path).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
