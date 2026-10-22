//! 安装升级的受控资料副本。业务安装与数据指针由同一个 `SQLite` 事务提交。
use super::{AppServerServices, ServiceError, persistence_error, validate_project_id};
use contexthub_persistence_sqlite::{PersonalApplicationDataRecord, PersonalApplicationDataSlot};
use contexthub_platform_adapter::{filesystem, security};
use contexthub_protocol::{
    PersonalApplicationDataAction as Action, PersonalApplicationDataExecuteParams as Execute,
    PersonalApplicationDataOperation as Operation, PersonalApplicationDataPrepareParams as Prepare,
    PersonalApplicationDataState as State,
};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read as _, Write as _},
    path::{Component, Path, PathBuf},
};

const MAX_FILE: u64 = 128 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 50_000;
type Snapshot = BTreeMap<String, Option<(u64, String)>>;
pub(super) struct Activation<'a> {
    pub client: &'a str,
    pub project: &'a str,
    pub digest: &'a str,
    pub revision: i64,
    pub action: Action,
}
fn relative_name(root: &Path, path: &Path) -> Result<String, ServiceError> {
    path.strip_prefix(root)
        .map_err(|_| error("personal_application_data_path_invalid"))?
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .filter(|s| !s.is_empty() && !s.contains(['\\', ':']))
                .map(str::to_owned)
                .ok_or_else(|| error("personal_application_data_path_invalid")),
            _ => Err(error("personal_application_data_path_invalid")),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}
fn error(reason: &'static str) -> ServiceError {
    ServiceError::new(reason)
}
fn ensure_directory(path: &Path) -> Result<(), ServiceError> {
    for ancestor in path.ancestors() {
        if fs::symlink_metadata(ancestor).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(error("personal_application_data_path_invalid"));
        }
    }
    security::ensure_restricted_directory(path)
        .map_err(|_| error("personal_application_data_write_failed"))
}
fn slot_path(profile: &Path, slot: &PersonalApplicationDataSlot) -> Result<PathBuf, ServiceError> {
    if !slot
        .relative_path
        .split('/')
        .all(|p| !p.is_empty() && p != "." && p != "..")
        || !Path::new(&slot.relative_path)
            .components()
            .all(|p| matches!(p, Component::Normal(_)))
    {
        return Err(error("personal_application_data_path_invalid"));
    }
    let path = profile.join(&slot.relative_path);
    for ancestor in path.ancestors() {
        if fs::symlink_metadata(ancestor).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(error("personal_application_data_path_invalid"));
        }
    }
    Ok(path)
}
fn file_bytes(path: &Path) -> Result<Vec<u8>, ServiceError> {
    let before = filesystem::symlink_metadata_with_stable_identity(path)
        .map_err(|_| error("personal_application_data_read_failed"))?;
    if !before.metadata.is_file()
        || before.metadata.file_type().is_symlink()
        || before.metadata.len() > MAX_FILE
        || filesystem::is_cloud_placeholder_file(&before.metadata)
    {
        return Err(error("personal_application_data_file_invalid"));
    }
    let mut file = filesystem::open_regular_file_no_follow(path, false)
        .map_err(|_| error("personal_application_data_read_failed"))?;
    let opened = filesystem::opened_file_metadata_with_stable_identity(&file)
        .map_err(|_| error("personal_application_data_read_failed"))?;
    if opened.stable_identity != before.stable_identity {
        return Err(error("personal_application_data_source_changed"));
    }
    let mut bytes = vec![];
    std::io::Read::by_ref(&mut file)
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("personal_application_data_read_failed"))?;
    let after = filesystem::symlink_metadata_with_stable_identity(path)
        .map_err(|_| error("personal_application_data_source_changed"))?;
    if after.stable_identity != before.stable_identity
        || after.metadata.len() != before.metadata.len()
        || after.metadata.modified().ok() != before.metadata.modified().ok()
        || bytes.len() as u64 != before.metadata.len()
    {
        return Err(error("personal_application_data_source_changed"));
    }
    Ok(bytes)
}
fn walk(
    root: &Path,
    directory: &Path,
    snapshot: &mut Snapshot,
    total: &mut u64,
    depth: usize,
) -> Result<(), ServiceError> {
    if depth > 32 || snapshot.len() > MAX_FILES {
        return Err(error("personal_application_data_snapshot_limit"));
    }
    let before = filesystem::symlink_metadata_with_stable_identity(directory)
        .map_err(|_| error("personal_application_data_source_missing"))?;
    if !before.metadata.is_dir() || before.metadata.file_type().is_symlink() {
        return Err(error("personal_application_data_path_invalid"));
    }
    for entry in
        fs::read_dir(directory).map_err(|_| error("personal_application_data_read_failed"))?
    {
        let path = entry
            .map_err(|_| error("personal_application_data_read_failed"))?
            .path();
        if snapshot.len() >= MAX_FILES {
            return Err(error("personal_application_data_snapshot_limit"));
        }
        let name = relative_name(root, &path)?;
        let meta = fs::symlink_metadata(&path)
            .map_err(|_| error("personal_application_data_read_failed"))?;
        if meta.file_type().is_symlink() {
            return Err(error("personal_application_data_path_invalid"));
        }
        if meta.is_dir() {
            snapshot.insert(name, None);
            walk(root, &path, snapshot, total, depth + 1)?;
        } else {
            let bytes = file_bytes(&path)?;
            *total = total
                .checked_add(bytes.len() as u64)
                .filter(|n| *n <= MAX_TOTAL)
                .ok_or_else(|| error("personal_application_data_snapshot_limit"))?;
            snapshot.insert(
                name,
                Some((
                    bytes.len() as u64,
                    format!("sha256:{:x}", Sha256::digest(&bytes)),
                )),
            );
        }
    }
    let after = filesystem::symlink_metadata_with_stable_identity(directory)
        .map_err(|_| error("personal_application_data_source_changed"))?;
    if after.stable_identity != before.stable_identity
        || after.metadata.modified().ok() != before.metadata.modified().ok()
    {
        return Err(error("personal_application_data_source_changed"));
    }
    Ok(())
}
fn snapshot(root: &Path) -> Result<Snapshot, ServiceError> {
    let mut value = BTreeMap::new();
    walk(root, root, &mut value, &mut 0, 0)?;
    Ok(value)
}
fn snapshot_digest(snapshot: &Snapshot) -> Result<String, ServiceError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(snapshot)
                .map_err(|_| error("personal_application_data_record_invalid"))?
        )
    ))
}
fn fork(profile: &Path, record: &PersonalApplicationDataRecord) -> Result<String, ServiceError> {
    let source = record
        .source_slot
        .as_ref()
        .map(|s| slot_path(profile, s))
        .transpose()?;
    if record.operation.action == Action::Rollback {
        let data = slot_path(profile, &record.target_slot)?;
        let digest = snapshot_digest(&snapshot(&data)?)?;
        if record.expected_retained_digest.as_ref() != Some(&digest) {
            return Err(error("personal_application_data_retained_changed"));
        }
        return Ok(digest);
    }
    let initial = source
        .as_ref()
        .map(|p| snapshot(p))
        .transpose()?
        .unwrap_or_default();
    let target = slot_path(profile, &record.target_slot)?;
    if target.exists() {
        return Err(error("personal_application_data_candidate_exists"));
    }
    ensure_directory(
        target
            .parent()
            .ok_or_else(|| error("personal_application_data_path_invalid"))?,
    )?;
    fs::create_dir(&target).map_err(|_| error("personal_application_data_write_failed"))?;
    ensure_directory(&target)?;
    for (relative, entry) in &initial {
        let destination = target.join(relative);
        if entry.is_none() {
            ensure_directory(&destination)?;
            continue;
        }
        let source = source
            .as_ref()
            .ok_or_else(|| error("personal_application_data_source_missing"))?;
        let bytes = file_bytes(&source.join(relative))?;
        ensure_directory(
            destination
                .parent()
                .ok_or_else(|| error("personal_application_data_path_invalid"))?,
        )?;
        let mut file = security::create_new_restricted_file(&destination)
            .map_err(|_| error("personal_application_data_write_failed"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| error("personal_application_data_write_failed"))?;
    }
    if snapshot(&target)? != initial
        || source
            .as_ref()
            .map(|p| snapshot(p))
            .transpose()?
            .unwrap_or_default()
            != initial
    {
        return Err(error("personal_application_data_source_changed"));
    }
    for (relative, entry) in initial.iter().rev() {
        if entry.is_none() {
            filesystem::sync_directory(&target.join(relative))
                .map_err(|_| error("personal_application_data_write_failed"))?;
        }
    }
    filesystem::sync_directory(&target)
        .and_then(|()| {
            filesystem::sync_directory(
                target
                    .parent()
                    .ok_or_else(|| std::io::Error::other("invalid parent"))?,
            )
        })
        .map_err(|_| error("personal_application_data_write_failed"))?;
    snapshot_digest(&initial)
}
impl AppServerServices {
    fn application_data_context(&self) -> Result<(PathBuf, String), ServiceError> {
        self.data_context()
    }
    pub(super) fn personal_application_data_runtime_root(
        &mut self,
        id: &str,
        digest: &str,
        now: i64,
    ) -> Result<PathBuf, ServiceError> {
        let (profile, profile_id) = self.application_data_context()?;
        self.persistence_mut()?
            .check_personal_application_data_admission(&profile_id)
            .map_err(persistence_error)?;
        let mut slot = self
            .persistence()?
            .personal_application_data_slot(&profile_id, id)
            .map_err(persistence_error)?;
        if slot.is_none() {
            ensure_directory(&profile.join("personal-capability-data").join(id))?;
            slot = Some(
                self.persistence_mut()?
                    .register_personal_application_legacy_data(&profile_id, id, digest, now)
                    .map_err(persistence_error)?,
            );
        }
        let slot = slot.ok_or_else(|| error("personal_application_data_slot_missing"))?;
        if slot.package_digest != digest {
            return Err(error("personal_application_data_source_changed"));
        }
        let path = slot_path(&profile, &slot)?;
        if !path.is_dir() {
            return Err(error("personal_application_data_source_missing"));
        }
        Ok(path)
    }
    pub(crate) fn personal_application_data_rpc(
        &mut self,
        method: &str,
        value: Value,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let (_, id) = self.application_data_context()?;
        let encode = |v: &Operation| {
            serde_json::to_value(v).map_err(|_| error("personal_application_data_record_invalid"))
        };
        if method == "personalApplicationData/prepare" {
            let p: Prepare = serde_json::from_value(value)
                .map_err(|_| error("personal_application_data_request_invalid"))?;
            return encode(&self.application_data_prepare(&p, now)?);
        }
        if method == "personalApplicationData/list" {
            return serde_json::to_value(
                self.persistence()?
                    .personal_application_data_operations(&id)
                    .map_err(persistence_error)?
                    .into_iter()
                    .map(|r| r.operation)
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| error("personal_application_data_record_invalid"));
        }
        if method == "personalApplicationData/resolveInterrupted" {
            let p: contexthub_protocol::PersonalApplicationDataConfirmParams =
                serde_json::from_value(value)
                    .map_err(|_| error("personal_application_data_request_invalid"))?;
            let r = self
                .persistence_mut()?
                .resolve_interrupted_application_data(&id, &p.operation_id, &p.preview_digest, now)
                .map_err(persistence_error)?;
            return encode(&r.operation);
        }
        if method == "personalApplicationData/rollback" {
            let p: Execute = serde_json::from_value(value)
                .map_err(|_| error("personal_application_data_request_invalid"))?;
            return self.application_data_rollback(&p, now);
        }
        if method == "personalApplicationData/cancel" {
            let p: contexthub_protocol::PersonalApplicationDataConfirmParams =
                serde_json::from_value(value)
                    .map_err(|_| error("personal_application_data_request_invalid"))?;
            let mut r = self
                .persistence()?
                .personal_application_data_operation(&id, &p.operation_id)
                .map_err(persistence_error)?;
            if r.operation.read_only_after_restore {
                return Err(error("personal_application_data_restored_read_only"));
            }
            if r.operation.preview_digest != p.preview_digest {
                return Err(error("personal_application_data_preview_changed"));
            }
            if !matches!(r.operation.state, State::Prepared | State::Incomplete) {
                return Err(error("personal_application_data_operation_unconfirmed"));
            }
            let old = r.operation.state.clone();
            r.operation.state = State::Cancelled;
            r.operation.updated_at = now;
            self.persistence_mut()?
                .transition_personal_application_data(&r, &old)
                .map_err(persistence_error)?;
            return encode(&r.operation);
        }
        if method == "personalApplicationData/get" {
            let p: contexthub_protocol::PersonalApplicationDataParams =
                serde_json::from_value(value)
                    .map_err(|_| error("personal_application_data_request_invalid"))?;
            return encode(
                &self
                    .persistence()?
                    .personal_application_data_operation(&id, &p.operation_id)
                    .map_err(persistence_error)?
                    .operation,
            );
        }
        Err(error("personal_application_data_method_unknown"))
    }
    fn application_data_prepare(
        &mut self,
        p: &Prepare,
        now: i64,
    ) -> Result<Operation, ServiceError> {
        let (profile, id) = self.application_data_context()?;
        self.persistence_mut()?
            .check_personal_application_data_admission(&id)
            .map_err(persistence_error)?;
        validate_project_id(&p.creation_project_id)?;
        if p.expected_revision < 1 {
            return Err(error("personal_application_data_request_invalid"));
        }
        super::validate_digest(
            &p.package_digest,
            "capability_creation_package_digest_invalid",
        )?;
        super::validate_text(&p.client_operation_id, 160, "client_operation_id_invalid")?;
        let installation = self
            .persistence()?
            .personal_capability_installation(&p.creation_project_id)
            .map_err(persistence_error)?;
        if let Some(installed) = installation {
            self.personal_application_data_runtime_root(
                &p.creation_project_id,
                &installed.package_digest,
                now,
            )?;
        } else {
            let legacy = profile
                .join("personal-capability-data")
                .join(&p.creation_project_id);
            if legacy.exists() {
                ensure_directory(&legacy)?;
                self.persistence_mut()?
                    .register_personal_application_legacy_data(
                        &id,
                        &p.creation_project_id,
                        &p.package_digest,
                        now,
                    )
                    .map_err(persistence_error)?;
            }
        }
        Ok(self
            .persistence_mut()?
            .prepare_personal_application_data(&id, p, now)
            .map_err(persistence_error)?
            .operation)
    }
    fn application_data_rollback(&mut self, p: &Execute, now: i64) -> Result<Value, ServiceError> {
        let (_, profile_id) = self.application_data_context()?;
        let r = self
            .persistence()?
            .personal_application_data_operation(&profile_id, &p.operation_id)
            .map_err(persistence_error)?;
        if r.operation.read_only_after_restore {
            return Err(error("personal_application_data_restored_read_only"));
        }
        if r.operation.action != Action::Rollback || r.operation.preview_digest != p.preview_digest
        {
            return Err(error("personal_application_data_preview_changed"));
        }
        if r.operation.state == State::Completed {
            return serde_json::to_value(r.operation)
                .map_err(|_| error("personal_application_data_record_invalid"));
        }
        let project = r
            .rollback_project
            .as_ref()
            .ok_or_else(|| error("personal_application_data_rollback_source_invalid"))?;
        let install = r
            .rollback_installation
            .as_ref()
            .ok_or_else(|| error("personal_application_data_rollback_source_invalid"))?;
        let path = self.personal_install_path(
            &r.operation.creation_project_id,
            &r.operation.package_digest,
        )?;
        if path
            != self
                .data_dir
                .as_ref()
                .ok_or_else(|| error("data_dir_unavailable"))?
                .join(&install.installed_relative_path)
        {
            return Err(error("personal_capability_install_path_invalid"));
        }
        super::validate_existing_install(
            &path,
            &r.operation.creation_project_id,
            &r.operation.package_digest,
            project.source_digest.as_deref().unwrap_or_default(),
        )?;
        super::validate_installed_source(
            &path,
            &project.target_type,
            project.source_digest.as_deref().unwrap_or_default(),
        )?;
        self.prepare_application_data_copy(
            p,
            &Activation {
                client: &r.operation.client_operation_id,
                project: &r.operation.creation_project_id,
                digest: &r.operation.package_digest,
                revision: r.operation.expected_revision,
                action: Action::Rollback,
            },
            now,
        )?;
        if let Err(failure) = self.persistence_mut()?.rollback_personal_application_data(
            &profile_id,
            &p.operation_id,
            now,
        ) {
            self.fail_application_data(
                &p.operation_id,
                "personal_application_data_activation_unconfirmed",
                now,
            )?;
            return Err(persistence_error(failure));
        }
        serde_json::to_value(
            self.persistence()?
                .personal_application_data_operation(&profile_id, &p.operation_id)
                .map_err(persistence_error)?
                .operation,
        )
        .map_err(|_| error("personal_application_data_record_invalid"))
    }
    pub(super) fn prepare_application_data_copy(
        &mut self,
        p: &Execute,
        activation: &Activation<'_>,
        now: i64,
    ) -> Result<PersonalApplicationDataRecord, ServiceError> {
        let (profile, id) = self.application_data_context()?;
        let mut r = self
            .persistence()?
            .personal_application_data_operation(&id, &p.operation_id)
            .map_err(persistence_error)?;
        if r.operation.read_only_after_restore {
            return Err(error("personal_application_data_restored_read_only"));
        }
        if r.operation.state != State::Prepared {
            return Err(error("personal_application_data_operation_unconfirmed"));
        }
        if r.operation.preview_digest != p.preview_digest
            || r.operation.client_operation_id != activation.client
            || r.operation.creation_project_id != activation.project
            || r.operation.package_digest != activation.digest
            || r.operation.expected_revision != activation.revision
            || r.operation.action != activation.action
        {
            return Err(error("personal_application_data_preview_changed"));
        }
        let result = (|| {
            let proof = p
                .quiescence
                .as_ref()
                .ok_or_else(|| error("personal_data_quiescence_unconfirmed"))?;
            self.persistence_mut()?
                .verify_application_data_quiescence(&id, &p.operation_id, proof)
                .map_err(persistence_error)?;
            r.operation.state = State::Applying;
            r.operation.updated_at = now;
            self.persistence_mut()?
                .transition_personal_application_data(&r, &State::Prepared)
                .map_err(persistence_error)?;
            r.operation.snapshot_digest = Some(fork(&profile, &r)?);
            self.persistence_mut()?
                .transition_personal_application_data(&r, &State::Applying)
                .map_err(persistence_error)?;
            self.persistence_mut()?
                .verify_application_data_quiescence(&id, &p.operation_id, proof)
                .map_err(persistence_error)?;
            Ok::<_, ServiceError>(())
        })();
        if let Err(reason) = result {
            self.fail_application_data(&p.operation_id, reason.reason, now)?;
            return Err(reason);
        }
        Ok(r)
    }
    pub(super) fn fail_application_data(
        &mut self,
        operation: &str,
        reason: &str,
        now: i64,
    ) -> Result<(), ServiceError> {
        let (_, id) = self.application_data_context()?;
        let mut r = self
            .persistence()?
            .personal_application_data_operation(&id, operation)
            .map_err(persistence_error)?;
        if matches!(r.operation.state, State::Prepared | State::Applying) {
            let previous = r.operation.state.clone();
            r.operation.state = State::Incomplete;
            r.operation.reason = Some(reason.into());
            r.operation.updated_at = now;
            self.persistence_mut()?
                .transition_personal_application_data(&r, &previous)
                .map_err(persistence_error)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod test_support {
    use super::*;
    use contexthub_protocol::{ModelExecutionOwner, PersonalDataQuiescence};
    use serde_json::json;
    pub(crate) fn profile_root(prefix: &str) -> PathBuf {
        std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("{prefix}-{}", uuid::Uuid::new_v4().simple()))
            .join("cn.cilai.personal.workbench/business/profiles")
            .join(uuid::Uuid::new_v4().simple().to_string())
    }
    pub(crate) fn ready_db(
        root: &Path,
        name: &str,
        instance: &str,
    ) -> contexthub_persistence_sqlite::SqlitePersistence {
        let mut db =
            contexthub_persistence_sqlite::SqlitePersistence::open(&root.join(name), "test")
                .unwrap();
        let id = root.file_name().unwrap().to_str().unwrap();
        db.begin_personal_profile_upgrade(id, instance).unwrap();
        db.finish_personal_profile_upgrade(id, instance, "test-manifest")
            .unwrap();
        db
    }
    pub(crate) fn guarded(
        services: &mut AppServerServices,
        request: &Prepare,
        instance: &str,
    ) -> Execute {
        let op: Operation = serde_json::from_value(
            services
                .personal_application_data_rpc(
                    "personalApplicationData/prepare",
                    serde_json::to_value(request).unwrap(),
                    1,
                )
                .unwrap(),
        )
        .unwrap();
        let owner = ModelExecutionOwner {
            instance_id: "application-data-test-host".into(),
            pid: std::process::id(),
            start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(
                std::process::id(),
            )
            .unwrap()
            .unwrap(),
        };
        fence(services, op, instance, owner)
    }
    pub(crate) fn fence(
        services: &mut AppServerServices,
        op: Operation,
        instance: &str,
        owner: ModelExecutionOwner,
    ) -> Execute {
        let db = services.persistence_mut().unwrap();
        let fence=db.personal_writer_rpc("personalMaintenance/acquire",json!({"operationId":op.operation_id,"previewDigest":op.preview_digest,"owner":owner}),&op.profile_id,instance,2).unwrap();
        let state = db
            .personal_writer_rpc(
                "personalMaintenance/drainStatus",
                json!({"fenceId":fence["fenceId"],"owner":owner}),
                &op.profile_id,
                instance,
                2,
            )
            .unwrap();
        assert_eq!(state["activeWriters"], json!([]));
        assert_eq!(state["activeModelLeases"], json!([]));
        Execute {
            operation_id: op.operation_id,
            preview_digest: op.preview_digest,
            quiescence: Some(PersonalDataQuiescence {
                protocol_version: 1,
                profile_id: op.profile_id,
                owner,
                token: fence["fenceId"].as_str().unwrap().into(),
                writer_registry_digest: state["writerRegistryDigest"].as_str().unwrap().into(),
                covered_writers: [
                    "foreground",
                    "automation",
                    "review",
                    "memory",
                    "page_ai",
                    "authentication",
                    "personal_package_mcp",
                    "user_mcp",
                    "attachments",
                    "local_operations",
                    "ledger",
                    "native_history",
                    "runtime_initialization",
                ]
                .map(String::from)
                .to_vec(),
                native_homes: vec![],
            }),
        }
    }
    pub(crate) fn release(services: &mut AppServerServices, execute: &Execute, instance: &str) {
        let proof = execute.quiescence.as_ref().unwrap();
        services
            .persistence_mut()
            .unwrap()
            .personal_writer_rpc(
                "personalMaintenance/release",
                json!({"fenceId":proof.token,"owner":proof.owner}),
                &proof.profile_id,
                instance,
                9,
            )
            .unwrap();
    }
    pub(crate) fn upgrade(
        services: &mut AppServerServices,
        mut request: Value,
        instance: &str,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let data = guarded(
            services,
            &Prepare {
                creation_project_id: request["creationProjectId"].as_str().unwrap().into(),
                expected_revision: request["expectedRevision"].as_i64().unwrap(),
                package_digest: request["packageDigest"].as_str().unwrap().into(),
                client_operation_id: request["clientOperationId"].as_str().unwrap().into(),
                action: Action::Upgrade,
                target_operation_id: None,
            },
            instance,
        );
        request["dataChange"] = serde_json::to_value(&data).unwrap();
        let result =
            services.personal_capability_upgrade(&serde_json::from_value(request).unwrap(), now);
        release(services, &data, instance);
        result
    }
    pub(crate) fn import_commit(
        services: &mut AppServerServices,
        request: &contexthub_protocol::PersonalPackageImportCommitParams,
        instance: &str,
        now: i64,
    ) -> Result<contexthub_protocol::PersonalPackageImportCommitResult, ServiceError> {
        let preview = services.personal_package_import_get(
            &contexthub_protocol::PersonalPackageImportParams {
                import_id: request.import_id.clone(),
            },
            now,
        )?;
        if preview.state != contexthub_protocol::PersonalPackageImportState::Prepared {
            return services.personal_package_import_commit(request, now);
        }
        let prior = services
            .persistence()?
            .personal_application_data_operations(&preview.profile_id)
            .map_err(persistence_error)?
            .into_iter()
            .find(|r| r.operation.client_operation_id == request.client_operation_id);
        if let Some(r) = prior.filter(|r| r.operation.state != State::Prepared) {
            let mut request = request.clone();
            request.data_change = Some(Execute {
                operation_id: r.operation.operation_id,
                preview_digest: r.operation.preview_digest,
                quiescence: None,
            });
            return services.personal_package_import_commit(&request, now);
        }
        let p = &preview;
        let data = guarded(
            services,
            &Prepare {
                creation_project_id: p.local_creation_project_id.clone(),
                expected_revision: p.expected_revision.unwrap_or(1),
                package_digest: p.local_package_digest.clone(),
                client_operation_id: request.client_operation_id.clone(),
                action: Action::ImportUpgrade,
                target_operation_id: None,
            },
            instance,
        );
        let mut request = request.clone();
        request.data_change = Some(data.clone());
        let result = services.personal_package_import_commit(&request, now);
        release(services, &data, instance);
        result
    }
}

#[cfg(test)]
mod tests;
