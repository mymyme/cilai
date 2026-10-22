use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    ModelExecutionLease, ModelExecutionLeaseParams, ModelPurpose, ModelUsageBeginParams,
    ModelUsageFinishParams, ModelUsageListParams, ModelUsageListResult, ModelUsageRecord,
    ModelUsageState,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

#[cfg(test)]
#[path = "model_usage_tests.rs"]
mod tests;

fn error(code: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, code, code)
}
fn request_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|id_value| {
        id_value.get_version_num() == 4 && id_value.simple().to_string() == id
    })
}
fn opaque(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn validate_finish(p: &ModelUsageFinishParams) -> Result<(), PersistenceError> {
    if !request_id(&p.request_id)
        || p.state == ModelUsageState::Started
        || p.response_id_sha256
            .as_ref()
            .is_some_and(|value| !sha256(value))
        || p.reason_code.as_ref().is_some_and(|value| {
            !matches!(
                value.as_str(),
                "usage_not_provided"
                    | "usage_invalid"
                    | "usage_observation_limit"
                    | "upstream_http_rejected"
                    | "stream_interrupted"
                    | "stream_ended_without_completion"
            )
        })
        || p.cost.as_ref().is_some_and(|cost| {
            let mut decimal = cost.amount.split('.');
            let whole = decimal.next().unwrap_or_default();
            let fraction = decimal.next();
            cost.source != "upstream.responses.usage.cost"
                || cost.currency.len() != 3
                || !cost.currency.bytes().all(|b| b.is_ascii_uppercase())
                || cost.amount.len() > 80
                || whole.is_empty()
                || !whole.bytes().all(|b| b.is_ascii_digit())
                || fraction.is_some_and(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
                || decimal.next().is_some()
        })
    {
        return Err(error("model_usage_request_invalid"));
    }
    Ok(())
}
fn read(db: &Connection, id: &str) -> Result<Option<ModelUsageRecord>, PersistenceError> {
    let raw: Option<String> = db.query_row(
        "SELECT payload_json FROM durable_events WHERE object_type='personal_model_usage' AND object_id=?1 ORDER BY global_sequence DESC LIMIT 1",
        [id], |row| row.get(0),
    ).optional().map_err(database_error("read model usage"))?;
    raw.map(|raw| decode(&raw)).transpose()
}
fn decode(raw: &str) -> Result<ModelUsageRecord, PersistenceError> {
    let record: ModelUsageRecord =
        serde_json::from_str(raw).map_err(|_| error("model_usage_record_invalid"))?;
    if !request_id(&record.request_id)
        || record.read_only_after_restore
        || !matches!(record.purpose, ModelPurpose::Review | ModelPurpose::Memory)
        || record.source_profile_id.len() != 32
        || record.finished_at.is_some() != (record.state != ModelUsageState::Started)
    {
        return Err(error("model_usage_record_invalid"));
    }
    Ok(record)
}
fn append(tx: &Transaction<'_>, r: &ModelUsageRecord, now: i64) -> Result<(), PersistenceError> {
    let stage = if r.state == ModelUsageState::Started {
        "started"
    } else {
        "finished"
    };
    let body = serde_json::to_string(r).map_err(|_| error("model_usage_record_invalid"))?;
    tx.execute("INSERT INTO durable_events(event_id,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at) VALUES(?1,?2,'personal_model_usage',?3,?4,?3,?5,?5)",
        params![format!("personal-model-usage:{}:{stage}",r.request_id),format!("personal_model.usage_{stage}"),r.request_id,body,now])
        .map_err(database_error("append model usage"))?;
    Ok(())
}
fn finish_params(r: &ModelUsageRecord) -> ModelUsageFinishParams {
    ModelUsageFinishParams {
        request_id: r.request_id.clone(),
        lease_id: r.lease_id.clone(),
        snapshot_digest: r.snapshot_digest.clone(),
        state: r.state.clone(),
        response_id_sha256: r.response_id_sha256.clone(),
        tokens: r.tokens.clone(),
        cost: r.cost.clone(),
        reason_code: r.reason_code.clone(),
    }
}

impl SqlitePersistence {
    /// # Errors
    /// 维护、原生执行授权变化或请求标识重复时，拒绝在发出请求前建立账本。
    pub fn begin_model_usage(
        &mut self,
        profile: &str,
        p: &ModelUsageBeginParams,
        now: i64,
    ) -> Result<ModelUsageRecord, PersistenceError> {
        if !request_id(&p.request_id) || !opaque(&p.scope_id) {
            return Err(error("model_usage_request_invalid"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin model usage"))?;
        let instance: String = tx.query_row("SELECT instance_fence FROM personal_profile_compatibility WHERE profile_id=?1 AND state='ready'",[profile],|row|row.get(0))
            .map_err(database_error("read model usage profile"))?;
        crate::profile_writers::require_writer_admission(&tx, profile, &instance)?;
        let lease: ModelExecutionLease =
            serde_json::from_value(crate::model_connections::validate_for_request(
                &tx,
                &ModelExecutionLeaseParams {
                    lease_id: p.lease_id.clone(),
                    snapshot_digest: p.snapshot_digest.clone(),
                },
                &instance,
                now,
            )?)
            .map_err(|_| error("model_usage_record_invalid"))?;
        if lease.snapshot.profile_id != profile
            || !matches!(
                lease.snapshot.purpose,
                ModelPurpose::Review | ModelPurpose::Memory
            )
        {
            return Err(error("model_usage_execution_mismatch"));
        }
        if read(&tx, &p.request_id)?.is_some() {
            return Err(error("model_usage_request_reused"));
        }
        let s = lease.snapshot;
        let record = ModelUsageRecord {
            request_id: p.request_id.clone(),
            source_profile_id: profile.into(),
            lease_id: p.lease_id.clone(),
            snapshot_digest: p.snapshot_digest.clone(),
            connection_id: s.connection_id,
            config_revision: s.config_revision,
            auth_generation: s.auth_generation,
            model_id: s.model_id,
            purpose: s.purpose,
            scope_id: p.scope_id.clone(),
            state: ModelUsageState::Started,
            response_id_sha256: None,
            tokens: None,
            cost: None,
            reason_code: None,
            started_at: now,
            finished_at: None,
            read_only_after_restore: false,
        };
        append(&tx, &record, now)?;
        tx.commit()
            .map_err(database_error("commit model usage start"))?;
        Ok(record)
    }

    /// # Errors
    /// 未受理、来源资料变化或终态冲突时拒绝；撤销连接后仍可保存已发生请求的回执。
    pub fn finish_model_usage(
        &mut self,
        profile: &str,
        p: &ModelUsageFinishParams,
        now: i64,
    ) -> Result<ModelUsageRecord, PersistenceError> {
        validate_finish(p)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin model usage finish"))?;
        let mut r = read(&tx, &p.request_id)?.ok_or_else(|| error("model_usage_not_found"))?;
        if r.source_profile_id != profile {
            return Err(error("model_usage_restored_read_only"));
        }
        if r.lease_id != p.lease_id || r.snapshot_digest != p.snapshot_digest {
            return Err(error("model_usage_execution_mismatch"));
        }
        if r.state != ModelUsageState::Started {
            return if finish_params(&r) == *p {
                Ok(r)
            } else {
                Err(error("model_usage_terminal_conflict"))
            };
        }
        r.state = p.state.clone();
        r.response_id_sha256.clone_from(&p.response_id_sha256);
        r.tokens.clone_from(&p.tokens);
        r.cost.clone_from(&p.cost);
        r.reason_code.clone_from(&p.reason_code);
        r.finished_at = Some(now);
        append(&tx, &r, now)?;
        tx.commit()
            .map_err(database_error("commit model usage finish"))?;
        Ok(r)
    }

    /// # Errors
    /// 标识非法、记录缺失或损坏时失败，不补造统计。
    pub fn model_usage(
        &self,
        profile: &str,
        id: &str,
    ) -> Result<ModelUsageRecord, PersistenceError> {
        if !request_id(id) {
            return Err(error("model_usage_request_invalid"));
        }
        let mut r = read(&self.connection, id)?.ok_or_else(|| error("model_usage_not_found"))?;
        r.read_only_after_restore = r.source_profile_id != profile;
        Ok(r)
    }

    /// # Errors
    /// 游标、页大小或持久元数据无效时失败。分页固定事件上界，不能跨时点重复统计。
    pub fn list_model_usage(
        &self,
        profile: &str,
        p: &ModelUsageListParams,
    ) -> Result<ModelUsageListResult, PersistenceError> {
        if p.limit == 0
            || p.limit > 500
            || p.before_sequence.is_some_and(|v| v <= 0)
            || p.through_sequence.is_some_and(|v| v < 0)
        {
            return Err(error("model_usage_request_invalid"));
        }
        let snapshot = p.through_sequence.unwrap_or(
            self.connection
                .query_row(
                    "SELECT COALESCE(MAX(global_sequence),0) FROM durable_events",
                    [],
                    |row| row.get(0),
                )
                .map_err(database_error("read usage event bound"))?,
        );
        let mut statement=self.connection.prepare("WITH latest AS (SELECT object_id,MAX(global_sequence) AS n FROM durable_events WHERE object_type='personal_model_usage' AND global_sequence<=?1 GROUP BY object_id) SELECT e.global_sequence,e.payload_json FROM latest l JOIN durable_events e ON e.global_sequence=l.n WHERE (?2 IS NULL OR e.global_sequence<?2) ORDER BY e.global_sequence DESC LIMIT ?3")
            .map_err(database_error("list model usage"))?;
        let rows = statement
            .query_map(params![snapshot, p.before_sequence, p.limit + 1], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error("list model usage"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read model usage page"))?;
        let more = rows.len() > p.limit as usize;
        let mut records = Vec::new();
        let mut last = None;
        for (seq, raw) in rows.into_iter().take(p.limit as usize) {
            let mut r = decode(&raw)?;
            r.read_only_after_restore = r.source_profile_id != profile;
            records.push(r);
            last = Some(seq);
        }
        Ok(ModelUsageListResult {
            records,
            snapshot_sequence: snapshot,
            next_before_sequence: if more { last } else { None },
        })
    }
}
