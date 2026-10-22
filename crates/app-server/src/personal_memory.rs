//! 记忆授权只读查询和跨当前资料全部受管原生作用域的显式删除协调。
use crate::{AppServerServices, ServiceError};
use contexthub_persistence_sqlite::{
    PersonalMemoryResetRecord, PersonalMemoryResetScopeAuthorization,
};
use contexthub_platform_adapter::filesystem;
use contexthub_protocol::{
    PersonalMemoryParams, PersonalMemoryResetConfirmParams, PersonalMemoryResetFinishParams,
    PersonalMemoryResetOperation, PersonalMemoryResetOutcome, PersonalMemoryResetParams,
    PersonalMemoryResetPermit, PersonalMemoryResetPrepareParams, PersonalMemoryResetScope,
    PersonalMemoryResetScopeCompleteParams, PersonalMemoryResetScopeResult,
    PersonalMemoryResetStartParams, PersonalMemoryResetStartResult, PersonalMemoryResetState,
    PersonalMemoryResetWriterParams,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
const MAX_INSPECTION_FILES: usize = 100_000;
fn error(code: &'static str) -> ServiceError {
    ServiceError::new(code)
}
fn db_error(e: &contexthub_persistence_sqlite::PersistenceError) -> ServiceError {
    error(e.reason().unwrap_or("personal_memory_reset_storage_failed"))
}
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ServiceError> {
    serde_json::from_value(value).map_err(|_| error("personal_memory_reset_request_invalid"))
}
fn hash(value: &impl Serialize) -> Result<String, ServiceError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(value).map_err(|_| error("personal_memory_reset_record_invalid"))?
        )
    ))
}
fn directory(path: &Path) -> Result<(), ServiceError> {
    for parent in path.ancestors() {
        let metadata = fs::symlink_metadata(parent)
            .map_err(|_| error("personal_memory_reset_scope_unavailable"))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || filesystem::is_cloud_placeholder_file(&metadata)
        {
            return Err(error("personal_memory_reset_scope_denied"));
        }
    }
    Ok(())
}
fn inspect_memory_tree(
    path: &Path,
    memory: bool,
    entries: &mut usize,
    count: &mut u32,
    bytes: &mut u64,
) -> Result<(), ServiceError> {
    *entries = entries
        .checked_add(1)
        .ok_or_else(|| error("personal_memory_reset_inspection_limit"))?;
    if *entries > MAX_INSPECTION_FILES {
        return Err(error("personal_memory_reset_inspection_limit"));
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| error("personal_memory_reset_scope_unavailable"))?;
    if metadata.file_type().is_symlink()
        || filesystem::is_cloud_placeholder_file(&metadata)
        || (!metadata.is_dir() && !metadata.is_file())
    {
        return Err(error("personal_memory_reset_scope_denied"));
    }
    if metadata.is_file() {
        if memory {
            *count = count
                .checked_add(1)
                .ok_or_else(|| error("personal_memory_reset_inspection_limit"))?;
            *bytes = bytes
                .checked_add(metadata.len())
                .ok_or_else(|| error("personal_memory_reset_inspection_limit"))?;
        }
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|_| error("personal_memory_reset_scope_unavailable"))? {
        let entry = entry.map_err(|_| error("personal_memory_reset_scope_unavailable"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| error("personal_memory_reset_scope_denied"))?;
        let memory = memory
            || matches!(
                name.as_str(),
                "memory"
                    | "memories"
                    | "memories_1.sqlite"
                    | "memories_1.sqlite-wal"
                    | "memories_1.sqlite-shm"
            );
        inspect_memory_tree(&entry.path(), memory, entries, count, bytes)?;
    }
    Ok(())
}
fn inspect_reset_scope(home: &Path, count: &mut u32, bytes: &mut u64) -> Result<(), ServiceError> {
    for entry in fs::read_dir(home).map_err(|_| error("personal_memory_reset_scope_unavailable"))? {
        let entry = entry.map_err(|_| error("personal_memory_reset_scope_unavailable"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|_| error("personal_memory_reset_scope_unavailable"))?;
        if metadata.file_type().is_symlink() || filesystem::is_cloud_placeholder_file(&metadata) {
            return Err(error("personal_memory_reset_scope_denied"));
        }
    }
    let mut entries = 0;
    for name in [
        "memories",
        "memories_v2",
        "memories_extensions",
        "memories_1.sqlite",
        "memories_1.sqlite-wal",
        "memories_1.sqlite-shm",
    ] {
        let path = home.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => inspect_memory_tree(&path, true, &mut entries, count, bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(error("personal_memory_reset_scope_unavailable")),
        }
    }
    Ok(())
}
fn scope_plan(
    profile: &Path,
    id: &str,
) -> Result<
    (
        Vec<PersonalMemoryResetScope>,
        Vec<PersonalMemoryResetScopeAuthorization>,
    ),
    ServiceError,
> {
    let homes = crate::personal_data::discover_native_homes(profile)?;
    let mut scopes = vec![];
    let mut authorizations = vec![];
    for relative in homes {
        let home = profile.join(&relative);
        directory(&home)?;
        let identity = filesystem::symlink_metadata_with_stable_identity(&home)
            .map_err(|_| error("personal_memory_reset_scope_unavailable"))?;
        let scope_id = format!(
            "memory_scope_{}",
            hash(&json!({"profileId":id,"nativeHome":relative}))?.trim_start_matches("sha256:")
        );
        let (mut count, mut bytes) = (0, 0);
        let measured = inspect_reset_scope(&home, &mut count, &mut bytes);
        let issue = measured.as_ref().err().map(|e| e.reason.into());
        let scope_kind = if relative.contains("/model-connections/") {
            if relative.contains("/application/") {
                "connection_application"
            } else {
                "connection_workbench"
            }
        } else if relative.contains("cilai-application-harness") {
            "default_application"
        } else {
            "default_workbench"
        };
        scopes.push(PersonalMemoryResetScope {
            scope_id: scope_id.clone(),
            native_home_relative_path: relative.clone(),
            scope_kind: scope_kind.into(),
            visible_memory_file_count: measured.as_ref().ok().map(|()| count),
            visible_memory_logical_bytes: measured.as_ref().ok().map(|()| bytes),
            inspection_issue: issue,
            retained_native_paths: Some(
                ["memory-publication", "memory"]
                    .into_iter()
                    .filter(|name| fs::symlink_metadata(home.join(name)).is_ok())
                    .map(|name| format!("{relative}/{name}"))
                    .collect(),
            ),
        });
        authorizations.push(PersonalMemoryResetScopeAuthorization {
            scope_id,
            native_home_relative_path: relative,
            native_home_fingerprint: identity.stable_identity,
            permit_token: None,
            writer_lease_id: None,
            writer_client_operation_id: None,
        });
    }
    Ok((scopes, authorizations))
}
impl AppServerServices {
    fn memory_reset_read(
        &self,
        profile: &str,
        id: &str,
    ) -> Result<PersonalMemoryResetRecord, ServiceError> {
        self.persistence
            .as_ref()
            .ok_or_else(|| error("persistence_unavailable"))?
            .personal_memory_reset_operation(profile, id)
            .map_err(|e| db_error(&e))?
            .ok_or_else(|| error("personal_memory_reset_operation_not_found"))
    }
    fn memory_reset_save(
        &mut self,
        record: &PersonalMemoryResetRecord,
        expected: &PersonalMemoryResetState,
    ) -> Result<PersonalMemoryResetRecord, ServiceError> {
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .transition_personal_memory_reset(record, expected)
            .map_err(|e| db_error(&e))
    }
    pub(crate) fn personal_memory_rpc(
        &mut self,
        method: &str,
        params: Value,
        instance: &str,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let (profile, id) = self.data_context()?;
        if method == "personalMemory/authorization" {
            let _: PersonalMemoryParams = parse(params)?;
            return self.memory_authorization_value();
        }
        let operation = match method {
            "personalMemoryReset/prepare" => {
                self.memory_reset_prepare(&profile, &id, parse(params)?, now)?
                    .operation
            }
            "personalMemoryReset/start" => {
                return serde_json::to_value(self.memory_reset_start(
                    &profile,
                    &id,
                    parse(params)?,
                    instance,
                    now,
                )?)
                .map_err(|_| error("personal_memory_reset_record_invalid"));
            }
            "personalMemoryReset/get" | "personalMemoryReset/reconcile" => {
                let p: PersonalMemoryResetParams = parse(params)?;
                self.memory_reset_read(&id, &p.operation_id)?.operation
            }
            "personalMemoryReset/list" => {
                let _: PersonalMemoryParams = parse(params)?;
                let operations = self
                    .persistence
                    .as_ref()
                    .ok_or_else(|| error("persistence_unavailable"))?
                    .personal_memory_reset_operations(&id)
                    .map_err(|e| db_error(&e))?
                    .into_iter()
                    .map(|record| record.operation)
                    .collect::<Vec<_>>();
                return serde_json::to_value(operations)
                    .map_err(|_| error("personal_memory_reset_record_invalid"));
            }
            "personalMemoryReset/prepareWriter" => {
                let p: PersonalMemoryResetWriterParams = parse(params)?;
                let result = self.memory_reset_prepare_writer(&profile, &id, &p, instance, now)?;
                return serde_json::to_value(result)
                    .map_err(|_| error("personal_memory_reset_record_invalid"));
            }
            "personalMemoryReset/scopeComplete" => {
                let p: PersonalMemoryResetScopeCompleteParams = parse(params)?;
                self.persistence
                    .as_mut()
                    .ok_or_else(|| error("persistence_unavailable"))?
                    .complete_personal_memory_reset_scope(&id, instance, &p, now)
                    .map_err(|e| db_error(&e))?
                    .operation
            }
            "personalMemoryReset/finish" => {
                let p: PersonalMemoryResetFinishParams = parse(params)?;
                self.persistence
                    .as_mut()
                    .ok_or_else(|| error("persistence_unavailable"))?
                    .finish_personal_memory_reset(
                        &id,
                        instance,
                        &p.operation_id,
                        &p.preview_digest,
                        &p.owner,
                        now,
                    )
                    .map_err(|e| db_error(&e))?
                    .operation
            }
            "personalMemoryReset/cancel" => {
                let p: PersonalMemoryResetConfirmParams = parse(params)?;
                let mut record = self.memory_reset_read(&id, &p.operation_id)?;
                if record.operation.preview_digest != p.preview_digest {
                    return Err(error("personal_memory_reset_preview_changed"));
                }
                if record.operation.state == PersonalMemoryResetState::Cancelled {
                    return serde_json::to_value(record.operation)
                        .map_err(|_| error("personal_memory_reset_record_invalid"));
                }
                if !matches!(
                    record.operation.state,
                    PersonalMemoryResetState::Prepared | PersonalMemoryResetState::Incomplete
                ) {
                    return Err(error("personal_memory_reset_operation_unconfirmed"));
                }
                let expected = record.operation.state.clone();
                record.operation.state = PersonalMemoryResetState::Cancelled;
                record.operation.updated_at = now;
                self.memory_reset_save(&record, &expected)?.operation
            }
            "personalMemoryReset/resolveInterrupted" => {
                self.memory_reset_resolve(&id, &parse(params)?, now)?
                    .operation
            }
            _ => return Err(error("personal_memory_reset_method_unknown")),
        };
        serde_json::to_value(operation).map_err(|_| error("personal_memory_reset_record_invalid"))
    }
    fn memory_authorization_value(&self) -> Result<Value, ServiceError> {
        serde_json::to_value(
            self.persistence
                .as_ref()
                .ok_or_else(|| error("persistence_unavailable"))?
                .personal_memory_authorization()
                .map_err(|e| db_error(&e))?,
        )
        .map_err(|_| error("personal_memory_reset_record_invalid"))
    }
    fn memory_reset_prepare_writer(
        &mut self,
        profile: &Path,
        id: &str,
        p: &PersonalMemoryResetWriterParams,
        instance: &str,
        now: i64,
    ) -> Result<contexthub_protocol::PersonalWriterLease, ServiceError> {
        let record = self.memory_reset_read(id, &p.permit.operation_id)?;
        let approved = record
            .scopes
            .iter()
            .find(|scope| {
                scope.scope_id == p.permit.scope_id
                    && scope.native_home_relative_path == p.permit.native_home_relative_path
                    && scope.native_home_fingerprint == p.permit.native_home_fingerprint
                    && scope.permit_token.as_ref() == Some(&p.permit.permit_token)
            })
            .ok_or_else(|| error("personal_memory_reset_permit_invalid"))?;
        let home = profile.join(&approved.native_home_relative_path);
        directory(&home)?;
        let (mut visible_count, mut visible_bytes) = (0, 0);
        inspect_reset_scope(&home, &mut visible_count, &mut visible_bytes)?;
        let metadata = filesystem::symlink_metadata_with_stable_identity(&home)
            .map_err(|_| error("personal_memory_reset_scope_unavailable"))?;
        if metadata.stable_identity != p.permit.native_home_fingerprint {
            return Err(error("personal_memory_reset_scope_changed"));
        }
        let result = self
            .persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .prepare_personal_memory_reset_writer(id, instance, p, now)
            .map_err(|e| db_error(&e))?;
        Ok(result)
    }
    fn memory_reset_prepare(
        &mut self,
        profile: &Path,
        id: &str,
        p: PersonalMemoryResetPrepareParams,
        now: i64,
    ) -> Result<PersonalMemoryResetRecord, ServiceError> {
        if p.client_operation_id.trim().is_empty() || p.client_operation_id.len() > 240 {
            return Err(error("personal_memory_reset_request_invalid"));
        }
        let (scopes, authorizations) = scope_plan(profile, id)?;
        let digest = hash(&json!({"profileId":id,"scopes":scopes,"homes":authorizations}))?;
        let blockers = scopes
            .iter()
            .filter_map(|scope| scope.inspection_issue.clone())
            .collect();
        let results = scopes
            .iter()
            .map(|scope| PersonalMemoryResetScopeResult {
                scope_id: scope.scope_id.clone(),
                outcome: PersonalMemoryResetOutcome::Pending,
                writer_lease_id: None,
                official_response_received: false,
                process_exit_confirmed: false,
                reason: None,
            })
            .collect();
        let record = PersonalMemoryResetRecord {
            operation: PersonalMemoryResetOperation {
                operation_id: format!("memory_reset_{}", uuid::Uuid::new_v4().simple()),
                profile_id: id.into(),
                state: PersonalMemoryResetState::Prepared,
                preview_digest: digest,
                created_at: now,
                updated_at: now,
                scopes,
                scope_results: results,
                blockers,
            },
            scopes: authorizations,
            owner: None,
            fence_id: None,
            request_fingerprint: hash(&p)?,
            client_operation_id: p.client_operation_id,
        };
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .prepare_personal_memory_reset(&record)
            .map_err(|e| db_error(&e))
    }
    fn memory_reset_start(
        &mut self,
        profile: &Path,
        id: &str,
        p: PersonalMemoryResetStartParams,
        instance: &str,
        now: i64,
    ) -> Result<PersonalMemoryResetStartResult, ServiceError> {
        let mut record = self.memory_reset_read(id, &p.operation_id)?;
        if record.operation.preview_digest != p.preview_digest {
            return Err(error("personal_memory_reset_preview_changed"));
        }
        if record.operation.state != PersonalMemoryResetState::Prepared {
            return Err(error("personal_memory_reset_operation_unconfirmed"));
        }
        let verified = match &p.quiescence {
            Some(proof) => self
                .persistence
                .as_mut()
                .ok_or_else(|| error("persistence_unavailable"))?
                .verify_personal_data_quiescence(id, &p.operation_id, instance, proof)
                .map_err(|e| db_error(&e)),
            None => Err(error("personal_data_quiescence_unconfirmed")),
        };
        let verification = verified.and_then(|()| {
            let (scopes, homes) = scope_plan(profile, id)?;
            if homes != record.scopes
                || scopes.iter().any(|scope| scope.inspection_issue.is_some())
                || !record.operation.blockers.is_empty()
            {
                return Err(error("personal_memory_reset_scope_changed"));
            }
            Ok(())
        });
        if let Err(reason) = verification {
            record.operation.state = PersonalMemoryResetState::Incomplete;
            record.operation.blockers = vec![reason.reason.into()];
            record.operation.updated_at = now;
            let record = self.memory_reset_save(&record, &PersonalMemoryResetState::Prepared)?;
            return Ok(PersonalMemoryResetStartResult {
                operation: record.operation,
                permits: vec![],
            });
        }
        let proof = p
            .quiescence
            .ok_or_else(|| error("personal_data_quiescence_unconfirmed"))?;
        record.owner = Some(proof.owner.clone());
        record.fence_id = Some(proof.token);
        record.operation.state = PersonalMemoryResetState::Applying;
        record.operation.updated_at = now;
        for scope in &mut record.scopes {
            scope.permit_token = Some(format!("reset_permit_{}", uuid::Uuid::new_v4().simple()));
        }
        let record = self.memory_reset_save(&record, &PersonalMemoryResetState::Prepared)?;
        let permits = record
            .scopes
            .iter()
            .map(|scope| {
                Ok(PersonalMemoryResetPermit {
                    operation_id: record.operation.operation_id.clone(),
                    preview_digest: record.operation.preview_digest.clone(),
                    scope_id: scope.scope_id.clone(),
                    native_home_relative_path: scope.native_home_relative_path.clone(),
                    native_home_fingerprint: scope.native_home_fingerprint.clone(),
                    permit_token: scope
                        .permit_token
                        .clone()
                        .ok_or_else(|| error("personal_memory_reset_record_invalid"))?,
                    owner: proof.owner.clone(),
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;
        Ok(PersonalMemoryResetStartResult {
            operation: record.operation,
            permits,
        })
    }
    fn memory_reset_resolve(
        &mut self,
        id: &str,
        p: &PersonalMemoryResetConfirmParams,
        now: i64,
    ) -> Result<PersonalMemoryResetRecord, ServiceError> {
        let mut record = self.memory_reset_read(id, &p.operation_id)?;
        if record.operation.preview_digest != p.preview_digest {
            return Err(error("personal_memory_reset_preview_changed"));
        }
        if record.operation.state != PersonalMemoryResetState::Applying {
            return Ok(record);
        }
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .verify_interrupted_personal_data(id, &p.operation_id)
            .map_err(|e| db_error(&e))?;
        for result in &mut record.operation.scope_results {
            if result.outcome != PersonalMemoryResetOutcome::Reset {
                result.outcome = PersonalMemoryResetOutcome::Unconfirmed;
                result.reason = Some("personal_memory_reset_response_unconfirmed".into());
            }
        }
        record.operation.state = if record.operation.scope_results.iter().all(|scope| {
            scope.outcome == PersonalMemoryResetOutcome::Reset
                && scope.official_response_received
                && scope.process_exit_confirmed
        }) {
            PersonalMemoryResetState::Completed
        } else {
            PersonalMemoryResetState::Incomplete
        };
        record.operation.updated_at = now;
        record.operation.blockers = record
            .operation
            .scope_results
            .iter()
            .filter_map(|result| result.reason.clone())
            .collect();
        self.memory_reset_save(&record, &PersonalMemoryResetState::Applying)
    }
}
