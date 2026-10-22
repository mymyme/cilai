use super::{
    ExecutionPreparation, ModelExecutionLease, ModelPurpose, PersistenceError,
    PersonalAutomationAdmitParams, PersonalAutomationAdmitResult, PersonalAutomationBindParams,
    PersonalAutomationBindResult, PersonalAutomationClaim, PersonalAutomationFinishOutcome,
    PersonalAutomationFinishParams, PersonalAutomationFinishResult,
    PersonalAutomationListTriggersParams, PersonalAutomationPrepareExecutionParams,
    PersonalAutomationResolveInterruptedParams, PersonalAutomationRuleRecord,
    PersonalAutomationTriggerItem, PersonalAutomationTriggerOutcome, PersonalAutomationTriggerPage,
    PersonalAutomationTriggerRecord, PersonalWriterLeaseParams, PersonalWriterMode,
    PersonalWriterPrepareParams, StoredTrigger, Transaction, Value, active_claim, append,
    current_claim, database_error, decode, digest, encode, json, model_lease, model_leases_ended,
    opaque, owned, owner_alive, params, read, reject, resource_id, rule, save_rule, sha256, uuid32,
};

fn validate_admit(p: &PersonalAutomationAdmitParams) -> Result<(), PersistenceError> {
    if !uuid32(&p.trigger_id) || !opaque(&p.rule_id) || !sha256(&p.expected_rule_digest) {
        return Err(reject("automation_trigger_request_invalid"));
    }
    Ok(())
}

fn version(source: &PersonalAutomationRuleRecord) -> Result<u32, PersistenceError> {
    source.value["value"]["ruleVersion"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| reject("automation_rule_record_invalid"))
}

fn schedule(
    source: &PersonalAutomationRuleRecord,
    p: &PersonalAutomationAdmitParams,
    now: i64,
) -> Result<(), PersistenceError> {
    let value = &source.value["value"];
    if p.scheduled_due_at.is_some_and(|at| at < 0 || at > now)
        || p.next_run_at.is_some_and(|at| at <= now)
    {
        return Err(reject("automation_trigger_request_invalid"));
    }
    if !value["modelId"].as_str().is_some_and(opaque) {
        return Err(reject("automation_model_required"));
    }
    if let Some(due) = p.scheduled_due_at {
        if value["enabled"] != true || value["nextRunAt"].as_i64() != Some(due) {
            return Err(reject("automation_schedule_changed"));
        }
        if p.next_run_at.is_none() {
            return Err(reject("automation_next_schedule_required"));
        }
    }
    Ok(())
}

fn advance_schedule(
    tx: &Transaction<'_>,
    source: &PersonalAutomationRuleRecord,
    p: &PersonalAutomationAdmitParams,
    busy: bool,
    now: i64,
) -> Result<PersonalAutomationRuleRecord, PersistenceError> {
    let mut record = source.clone();
    if p.scheduled_due_at.is_some() || !busy {
        record.value["value"]["nextRunAt"] = json!(p.next_run_at);
    }
    record.value["value"]["updatedAt"] = json!(now);
    record.value["value"]["lastError"] = if busy {
        json!("automation_previous_run_active")
    } else {
        Value::Null
    };
    save_rule(tx, &record, now)
}

fn acquire_claim(
    tx: &Transaction<'_>,
    p: &PersonalAutomationAdmitParams,
    source: &PersonalAutomationRuleRecord,
    profile: &str,
    fence: &str,
    now: i64,
) -> Result<PersonalAutomationClaim, PersistenceError> {
    let writer = crate::profile_writers::prepare_writer(
        tx,
        &PersonalWriterPrepareParams {
            client_operation_id: format!("automation:{}", p.trigger_id),
            kind: "local_operation".into(),
            resource_id: resource_id(&p.rule_id),
            resource_revision: source.digest.clone(),
            owner: p.owner.clone(),
            launch_protocol: 1,
            process_mode: PersonalWriterMode::RemoteRequest,
        },
        profile,
        fence,
        now,
    )?;
    Ok(PersonalAutomationClaim {
        claim_id: p.trigger_id.clone(),
        profile_id: profile.into(),
        rule_id: p.rule_id.clone(),
        rule_version: version(source)?,
        trigger_id: p.trigger_id.clone(),
        owner: p.owner.clone(),
        writer_lease_id: writer.lease_id,
        run_id: None,
        model_lease_ids: vec![],
        released: false,
    })
}

fn new_trigger(
    p: PersonalAutomationAdmitParams,
    source: PersonalAutomationRuleRecord,
    current: PersonalAutomationRuleRecord,
    claim: Option<PersonalAutomationClaim>,
    context: (&str, &str, i64),
) -> Result<StoredTrigger, PersistenceError> {
    let (profile, fence, now) = context;
    let busy = claim.is_none();
    let fingerprint = digest(&p)?;
    let version = version(&source)?;
    Ok(StoredTrigger {
        request_fingerprint: fingerprint,
        instance_fence: fence.into(),
        admission_rule_record: source,
        rule_record: current,
        trigger: PersonalAutomationTriggerRecord {
            trigger_id: p.trigger_id,
            claim_id: claim.as_ref().map(|c| c.claim_id.clone()),
            profile_id: profile.into(),
            rule_id: p.rule_id,
            rule_version: version,
            scheduled_due_at: p.scheduled_due_at,
            created_at: now,
            finished_at: busy.then_some(now),
            run_id: None,
            outcome: if busy {
                PersonalAutomationTriggerOutcome::Skipped
            } else {
                PersonalAutomationTriggerOutcome::Admitted
            },
            reason_code: busy.then(|| "automation_previous_run_active".into()),
            read_only_after_restore: false,
        },
        claim,
        preparations: vec![],
    })
}

fn admitted(record: StoredTrigger, replayed: bool) -> PersonalAutomationAdmitResult {
    PersonalAutomationAdmitResult {
        trigger: record.trigger,
        claim: record.claim,
        admission_rule_record: record.admission_rule_record,
        rule_record: record.rule_record,
        replayed,
    }
}

pub(super) fn admit(
    tx: &Transaction<'_>,
    p: PersonalAutomationAdmitParams,
    profile: &str,
    fence: &str,
    now: i64,
) -> Result<PersonalAutomationAdmitResult, PersistenceError> {
    validate_admit(&p)?;
    if let Some(old) = read(tx, &p.trigger_id)? {
        if old.request_fingerprint != digest(&p)? {
            return Err(reject("automation_trigger_id_reused"));
        }
        if old.trigger.profile_id != profile {
            return Err(reject("automation_claim_restored_read_only"));
        }
        return Ok(admitted(old, true));
    }
    crate::profile_writers::require_writer_admission(tx, profile, fence)?;
    if !owner_alive(&p.owner)? {
        return Err(reject("automation_claim_owner_unconfirmed"));
    }
    let source = rule(tx, &p.rule_id)?.ok_or_else(|| reject("automation_rule_not_found"))?;
    let has_claim = active_claim(tx, profile, &p.rule_id)?;
    let busy = has_claim
        || crate::profile_writers::active_writers(tx, profile)?
            .iter()
            .any(|lease| lease.resource_id == resource_id(&p.rule_id));
    if source.digest != p.expected_rule_digest {
        if !has_claim {
            return Err(reject("automation_rule_changed"));
        }
        let record = new_trigger(p, source.clone(), source, None, (profile, fence, now))?;
        append(tx, &record, now)?;
        return Ok(admitted(record, false));
    }
    schedule(&source, &p, now)?;
    let claim = if busy {
        None
    } else {
        Some(acquire_claim(tx, &p, &source, profile, fence, now)?)
    };
    let current = advance_schedule(tx, &source, &p, busy, now)?;
    let record = new_trigger(p, source, current, claim, (profile, fence, now))?;
    append(tx, &record, now)?;
    Ok(admitted(record, false))
}

pub(super) fn prepare_execution(
    tx: &Transaction<'_>,
    p: PersonalAutomationPrepareExecutionParams,
    profile: &str,
    fence: &str,
    now: i64,
) -> Result<ModelExecutionLease, PersistenceError> {
    if !opaque(&p.client_operation_id)
        || p.execution.owner.as_ref() != Some(&p.owner)
        || !matches!(
            p.execution.purpose,
            ModelPurpose::Automation | ModelPurpose::Review | ModelPurpose::Memory
        )
    {
        return Err(reject("automation_execution_request_invalid"));
    }
    let mut record = read(tx, &p.claim_id)?.ok_or_else(|| reject("automation_claim_not_found"))?;
    owned(tx, &record, profile, fence, &p.owner)?;
    let fingerprint = digest(&p)?;
    if let Some(old) = record
        .preparations
        .iter()
        .find(|item| item.client_operation_id == p.client_operation_id)
    {
        if old.fingerprint != fingerprint {
            return Err(reject("automation_execution_operation_reused"));
        }
        return model_lease(tx, &old.lease_id);
    }
    crate::profile_writers::require_writer_admission(tx, profile, fence)?;
    if record.preparations.len() >= 64 {
        return Err(reject("automation_execution_limit"));
    }
    let lease: ModelExecutionLease = decode(crate::model_connections::execution_prepare(
        tx,
        p.execution,
        profile,
        fence,
        now,
    )?)?;
    let claim = record
        .claim
        .as_mut()
        .ok_or_else(|| reject("automation_claim_not_found"))?;
    if lease.owner != claim.owner || lease.snapshot.profile_id != profile {
        return Err(reject("automation_execution_scope_changed"));
    }
    claim.model_lease_ids.push(lease.lease_id.clone());
    record.preparations.push(ExecutionPreparation {
        client_operation_id: p.client_operation_id,
        fingerprint,
        lease_id: lease.lease_id.clone(),
    });
    append(tx, &record, now)?;
    Ok(lease)
}

fn merge_rule(
    tx: &Transaction<'_>,
    record: &StoredTrigger,
    bind: bool,
    now: i64,
) -> Result<Option<PersonalAutomationRuleRecord>, PersistenceError> {
    let Some(mut current) = rule(tx, &record.trigger.rule_id)? else {
        return Ok(None);
    };
    if bind {
        current.value["value"]["lastRunId"] = json!(record.trigger.run_id);
        current.value["value"]["lastRunAt"] = json!(now);
    } else {
        current.value["value"]["lastError"] = json!(record.trigger.reason_code);
        if record.trigger.outcome == PersonalAutomationTriggerOutcome::OutcomeUnconfirmed
            && record.claim.as_ref().is_some_and(|claim| claim.released)
        {
            current.value["value"]["enabled"] = Value::Bool(false);
            current.value["value"]["nextRunAt"] = Value::Null;
        }
    }
    current.value["value"]["updatedAt"] = json!(now);
    save_rule(tx, &current, now).map(Some)
}

pub(super) fn bind(
    tx: &Transaction<'_>,
    p: PersonalAutomationBindParams,
    profile: &str,
    fence: &str,
    now: i64,
) -> Result<PersonalAutomationBindResult, PersistenceError> {
    if !opaque(&p.run_id) {
        return Err(reject("automation_run_id_invalid"));
    }
    let mut record = read(tx, &p.claim_id)?.ok_or_else(|| reject("automation_claim_not_found"))?;
    owned(tx, &record, profile, fence, &p.owner)?;
    let claim = record
        .claim
        .as_mut()
        .ok_or_else(|| reject("automation_claim_not_found"))?;
    if let Some(id) = &claim.run_id {
        if id != &p.run_id {
            return Err(reject("automation_claim_run_changed"));
        }
        return Ok(PersonalAutomationBindResult {
            claim: claim.clone(),
            rule_record: rule(tx, &record.trigger.rule_id)?,
        });
    }
    if tx.query_row("SELECT EXISTS(SELECT 1 FROM durable_events WHERE object_type='personal_automation_trigger' AND object_id<>?1 AND json_extract(payload_json,'$.claim.runId')=?2)",params![p.claim_id,p.run_id],|row|row.get::<_,bool>(0)).map_err(database_error("read automation run ownership"))? {
        return Err(reject("automation_run_already_bound"));
    }
    claim.run_id = Some(p.run_id.clone());
    record.trigger.run_id = Some(p.run_id);
    let current = merge_rule(tx, &record, true, now)?;
    append(tx, &record, now)?;
    Ok(PersonalAutomationBindResult {
        claim: record
            .claim
            .ok_or_else(|| reject("automation_claim_not_found"))?,
        rule_record: current,
    })
}

fn run_ended(
    tx: &Transaction<'_>,
    claim: &PersonalAutomationClaim,
    outcome: &PersonalAutomationFinishOutcome,
) -> Result<(), PersistenceError> {
    let Some(id) = &claim.run_id else {
        if *outcome == PersonalAutomationFinishOutcome::Completed
            || claim
                .model_lease_ids
                .iter()
                .map(|id| model_lease(tx, id))
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .any(|lease| lease.execution_started)
        {
            return Err(reject("automation_run_terminal_unconfirmed"));
        }
        return Ok(());
    };
    let raw: String = tx
        .query_row(
            "SELECT record_json FROM ai_run_ledger WHERE record_id=?1",
            [id],
            |row| row.get(0),
        )
        .map_err(database_error("read automation terminal run"))?;
    let run: Value =
        serde_json::from_str(&raw).map_err(|_| reject("automation_run_record_invalid"))?;
    if *outcome == PersonalAutomationFinishOutcome::Completed {
        let leases = claim
            .model_lease_ids
            .iter()
            .map(|id| model_lease(tx, id))
            .collect::<Result<Vec<_>, _>>()?;
        if !leases.iter().any(|lease| {
            lease.snapshot.purpose == ModelPurpose::Automation
                && lease.execution_started
                && encode(&lease.snapshot).is_ok_and(|snapshot| snapshot == run["modelExecution"])
        }) {
            return Err(reject("automation_run_execution_unconfirmed"));
        }
    }
    let expected = match outcome {
        PersonalAutomationFinishOutcome::Completed => "completed",
        PersonalAutomationFinishOutcome::Failed => "failed",
        PersonalAutomationFinishOutcome::Cancelled => "cancelled",
        PersonalAutomationFinishOutcome::OutcomeUnconfirmed => {
            return Err(reject("automation_run_terminal_unconfirmed"));
        }
    };
    if run["runId"] != *id
        || run["automationRuleId"] != claim.rule_id
        || run["automationRuleVersion"].as_u64() != Some(u64::from(claim.rule_version))
        || !(run["status"] == expected
            || (*outcome == PersonalAutomationFinishOutcome::Cancelled
                && run["status"] == "terminated"))
        || run["completedAt"].as_i64().is_none()
        || run["threadWriterActive"] != false
        || run["errorReason"].as_str().is_some_and(|code| {
            [
                "unconfirmed",
                "uncertain",
                "outcome_unknown",
                "response_invalid",
                "response_missing",
            ]
            .iter()
            .any(|marker| code.contains(marker))
        })
    {
        return Err(reject("automation_run_terminal_unconfirmed"));
    }
    let pending: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM ai_runtime_entries WHERE record_id GLOB 'permissionRequests:*' AND json_extract(record_json,'$.value.runId')=?1 AND (json_extract(record_json,'$.value.status')='pending' OR json_extract(record_json,'$.value.applicationWrite.executionStatus') IN ('submitting','outcome_unknown')))",[id],|row|row.get(0)).map_err(database_error("read automation pending permissions"))?;
    if pending {
        return Err(reject("automation_permission_outcome_unconfirmed"));
    }
    Ok(())
}

fn result(
    tx: &Transaction<'_>,
    record: StoredTrigger,
) -> Result<PersonalAutomationFinishResult, PersistenceError> {
    Ok(PersonalAutomationFinishResult {
        claim: record
            .claim
            .ok_or_else(|| reject("automation_claim_not_found"))?,
        trigger: record.trigger.clone(),
        rule_record: rule(tx, &record.trigger.rule_id)?,
    })
}

pub(super) fn finish(
    tx: &Transaction<'_>,
    p: PersonalAutomationFinishParams,
    profile: &str,
    fence: &str,
    now: i64,
) -> Result<PersonalAutomationFinishResult, PersistenceError> {
    if p.reason_code.as_ref().is_some_and(|code| !opaque(code)) {
        return Err(reject("automation_reason_invalid"));
    }
    let mut record = read(tx, &p.claim_id)?.ok_or_else(|| reject("automation_claim_not_found"))?;
    let target = match p.outcome {
        PersonalAutomationFinishOutcome::Completed => PersonalAutomationTriggerOutcome::Completed,
        PersonalAutomationFinishOutcome::Failed => PersonalAutomationTriggerOutcome::Failed,
        PersonalAutomationFinishOutcome::Cancelled => PersonalAutomationTriggerOutcome::Cancelled,
        PersonalAutomationFinishOutcome::OutcomeUnconfirmed => {
            PersonalAutomationTriggerOutcome::OutcomeUnconfirmed
        }
    };
    let claim = current_claim(&record, profile, fence)?;
    if claim.owner != p.owner || !owner_alive(&p.owner)? {
        return Err(reject("automation_claim_owner_unconfirmed"));
    }
    if claim.released {
        if record.trigger.outcome != target || record.trigger.reason_code != p.reason_code {
            return Err(reject("automation_claim_terminal_conflict"));
        }
        return result(tx, record);
    }
    if p.outcome == PersonalAutomationFinishOutcome::OutcomeUnconfirmed {
        if record.trigger.outcome != target || record.trigger.reason_code != p.reason_code {
            record.trigger.outcome = target;
            record.trigger.reason_code = p.reason_code;
            append(tx, &record, now)?;
        }
        return result(tx, record);
    }
    if record.trigger.outcome == PersonalAutomationTriggerOutcome::OutcomeUnconfirmed {
        return Err(reject("automation_claim_outcome_unconfirmed"));
    }
    model_leases_ended(tx, claim)?;
    run_ended(tx, claim, &p.outcome)?;
    crate::profile_writers::release_writer(
        tx,
        &PersonalWriterLeaseParams {
            lease_id: claim.writer_lease_id.clone(),
            owner: Some(claim.owner.clone()),
            local_transport_finished: true,
            remote_outcome_unconfirmed: false,
        },
        profile,
        fence,
        now,
    )?;
    record
        .claim
        .as_mut()
        .ok_or_else(|| reject("automation_claim_not_found"))?
        .released = true;
    record.trigger.outcome = target;
    record.trigger.reason_code = p.reason_code;
    record.trigger.finished_at = Some(now);
    merge_rule(tx, &record, false, now)?;
    append(tx, &record, now)?;
    result(tx, record)
}

pub(super) fn resolve(
    tx: &Transaction<'_>,
    p: &PersonalAutomationResolveInterruptedParams,
    profile: &str,
    fence: &str,
    now: i64,
) -> Result<PersonalAutomationFinishResult, PersistenceError> {
    let mut record = read(tx, &p.claim_id)?.ok_or_else(|| reject("automation_claim_not_found"))?;
    if record.trigger.profile_id != profile {
        return Err(reject("automation_claim_restored_read_only"));
    }
    let claim = record
        .claim
        .as_ref()
        .ok_or_else(|| reject("automation_claim_not_found"))?;
    if claim.released {
        return result(tx, record);
    }
    if owner_alive(&claim.owner)? {
        return Err(reject("automation_claim_owner_alive"));
    }
    for id in &claim.model_lease_ids {
        let lease = model_lease(tx, id)?;
        if lease.owner != claim.owner || lease.snapshot.profile_id != profile {
            return Err(reject("automation_claim_lease_invalid"));
        }
        crate::model_connections::recover_automation_lease(tx, &lease, now)?;
    }
    model_leases_ended(tx, claim)?;
    crate::profile_writers::recover_writer(tx, &claim.writer_lease_id, profile, fence, now)?;
    record
        .claim
        .as_mut()
        .ok_or_else(|| reject("automation_claim_not_found"))?
        .released = true;
    record.trigger.outcome = PersonalAutomationTriggerOutcome::OutcomeUnconfirmed;
    record.trigger.reason_code = Some("automation_interrupted_outcome_unconfirmed".into());
    record.trigger.finished_at = Some(now);
    merge_rule(tx, &record, false, now)?;
    append(tx, &record, now)?;
    result(tx, record)
}

pub(super) fn list(
    tx: &Transaction<'_>,
    p: &PersonalAutomationListTriggersParams,
    profile: &str,
) -> Result<PersonalAutomationTriggerPage, PersistenceError> {
    if !(1..=200).contains(&p.limit)
        || p.before_sequence.is_some_and(|s| s <= 0)
        || p.rule_id.as_ref().is_some_and(|id| !opaque(id))
    {
        return Err(reject("automation_trigger_list_invalid"));
    }
    let mut statement=tx.prepare("WITH latest AS (SELECT MAX(global_sequence) n FROM durable_events WHERE object_type='personal_automation_trigger' GROUP BY object_id) SELECT e.global_sequence,json_extract(e.payload_json,'$.trigger'),json_extract(e.payload_json,'$.claim.released') FROM latest l JOIN durable_events e ON e.global_sequence=l.n WHERE (?1 IS NULL OR json_extract(e.payload_json,'$.trigger.ruleId')=?1) AND (?2 IS NULL OR e.global_sequence<?2) ORDER BY e.global_sequence DESC LIMIT ?3").map_err(database_error("list automation triggers"))?;
    let rows = statement
        .query_map(
            params![p.rule_id, p.before_sequence, i64::from(p.limit) + 1],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<bool>>(2)?,
                ))
            },
        )
        .map_err(database_error("query automation triggers"))?;
    let mut items = vec![];
    for row in rows {
        let (sequence, raw, claim_released) =
            row.map_err(database_error("read automation trigger row"))?;
        let mut record: PersonalAutomationTriggerRecord =
            serde_json::from_str(&raw).map_err(|_| reject("automation_claim_record_invalid"))?;
        if record.read_only_after_restore || !uuid32(&record.trigger_id) {
            return Err(reject("automation_claim_record_invalid"));
        }
        record.read_only_after_restore = record.profile_id != profile;
        if record.claim_id.is_some() != claim_released.is_some() {
            return Err(reject("automation_claim_record_invalid"));
        }
        items.push(PersonalAutomationTriggerItem {
            sequence,
            record,
            claim_released,
        });
    }
    let more = items.len() > p.limit as usize;
    items.truncate(p.limit as usize);
    let next_before_sequence = more
        .then(|| items.last().map(|item| item.sequence))
        .flatten();
    Ok(PersonalAutomationTriggerPage {
        items,
        next_before_sequence,
    })
}
