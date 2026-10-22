//! 只协调个人应用命名空间固定日志段和既有浏览器 HTTP 缓存，不接收任意删除路径。
use crate::{AppServerServices, ServiceError};
use contexthub_persistence_sqlite::{
    PersonalNamespaceActionAuthorization, PersonalNamespaceStorageRecord,
};
use contexthub_platform_adapter::{filesystem, security};
use contexthub_protocol::{
    PersonalNamespaceActionReceipt, PersonalNamespaceActionState, PersonalNamespaceLogFile,
    PersonalNamespaceStorageKind, PersonalNamespaceStorageOperation,
    PersonalNamespaceStoragePrepareParams, PersonalNamespaceStorageStartParams,
    PersonalStorageOperationState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
fn error(code: &'static str) -> ServiceError {
    ServiceError::new(code)
}
fn db_error(e: &contexthub_persistence_sqlite::PersistenceError) -> ServiceError {
    error(e.reason().unwrap_or("personal_namespace_storage_failed"))
}
fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ServiceError> {
    serde_json::from_value(v).map_err(|_| error("personal_namespace_storage_request_invalid"))
}
fn value(v: impl Serialize) -> Result<Value, ServiceError> {
    serde_json::to_value(v).map_err(|_| error("personal_namespace_storage_record_invalid"))
}
fn hash(v: &impl Serialize) -> Result<String, ServiceError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(v)
                .map_err(|_| error("personal_namespace_storage_record_invalid"))?
        )
    ))
}
fn hex_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn directory(path: &Path) -> Result<(), ServiceError> {
    for p in path.ancestors() {
        let m = fs::symlink_metadata(p)
            .map_err(|_| error("personal_namespace_storage_namespace_unavailable"))?;
        if !m.is_dir() || m.file_type().is_symlink() || filesystem::is_cloud_placeholder_file(&m) {
            return Err(error("personal_namespace_storage_path_denied"));
        }
    }
    Ok(())
}
fn namespace(profile: &Path, id: &str) -> Result<PathBuf, ServiceError> {
    let profiles = profile
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "profiles"))
        .ok_or_else(|| error("personal_namespace_storage_namespace_invalid"))?;
    let business = profiles
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "business"))
        .ok_or_else(|| error("personal_namespace_storage_namespace_invalid"))?;
    let root = business
        .parent()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n == "cn.cilai.personal.workbench")
        })
        .ok_or_else(|| error("personal_namespace_storage_namespace_invalid"))?;
    if !hex_id(id) || profile.file_name().and_then(|n| n.to_str()) != Some(id) {
        return Err(error("personal_namespace_storage_namespace_invalid"));
    }
    directory(root)?;
    Ok(root.into())
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LogSegmentManifest {
    schema_version: u32,
    segment_id: String,
    profile_id: String,
    created_at: i64,
}
fn read_private<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ServiceError> {
    security::verify_current_user_private_file(path)
        .map_err(|_| error("personal_namespace_storage_journal_invalid"))?;
    let file = filesystem::open_regular_file_no_follow(path, false)
        .map_err(|_| error("personal_namespace_storage_journal_invalid"))?;
    if file
        .metadata()
        .map_err(|_| error("personal_namespace_storage_journal_invalid"))?
        .len()
        > 16384
    {
        return Err(error("personal_namespace_storage_journal_invalid"));
    }
    let mut data = vec![];
    file.take(16385)
        .read_to_end(&mut data)
        .map_err(|_| error("personal_namespace_storage_journal_invalid"))?;
    serde_json::from_slice(&data).map_err(|_| error("personal_namespace_storage_journal_invalid"))
}
fn file_name(file: &PersonalNamespaceLogFile) -> &'static str {
    match file {
        PersonalNamespaceLogFile::HostServiceCurrent => "host-service.log",
        PersonalNamespaceLogFile::HostServicePrevious => "host-service.log.1",
        PersonalNamespaceLogFile::TestDiagnosticsCurrent => "events.log",
        PersonalNamespaceLogFile::TestDiagnosticsPrevious => "events.log.1",
    }
}
fn selected_log_files(
    kinds: &[PersonalNamespaceStorageKind],
) -> BTreeSet<PersonalNamespaceLogFile> {
    kinds
        .iter()
        .flat_map(|k| match k {
            PersonalNamespaceStorageKind::HostServiceLogs => vec![
                PersonalNamespaceLogFile::HostServiceCurrent,
                PersonalNamespaceLogFile::HostServicePrevious,
            ],
            PersonalNamespaceStorageKind::TestDiagnosticsLogs => vec![
                PersonalNamespaceLogFile::TestDiagnosticsCurrent,
                PersonalNamespaceLogFile::TestDiagnosticsPrevious,
            ],
            PersonalNamespaceStorageKind::BrowserHttpCache => vec![],
        })
        .collect()
}
fn validate_segment(
    root: &Path,
    id: &str,
    p: &PersonalNamespaceStoragePrepareParams,
) -> Result<(), ServiceError> {
    let logs = p
        .item_kinds
        .iter()
        .any(|k| *k != PersonalNamespaceStorageKind::BrowserHttpCache);
    let Some(segment_id) = &p.log_segment_id else {
        if logs || !p.log_files.is_empty() {
            return Err(error("personal_namespace_storage_segment_invalid"));
        }
        return Ok(());
    };
    if !logs || !hex_id(segment_id) {
        return Err(error("personal_namespace_storage_segment_invalid"));
    }
    let segment = root.join("personal-storage-log-segments").join(segment_id);
    directory(&segment)?;
    let manifest: LogSegmentManifest = read_private(&segment.join("segment.json"))?;
    if manifest.schema_version != 1
        || manifest.segment_id != *segment_id
        || manifest.profile_id != id
        || manifest.created_at < 0
    {
        return Err(error("personal_namespace_storage_segment_invalid"));
    }
    for child in fs::read_dir(&segment)
        .map_err(|_| error("personal_namespace_storage_segment_unavailable"))?
    {
        let child = child.map_err(|_| error("personal_namespace_storage_segment_unavailable"))?;
        let name = child.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| error("personal_namespace_storage_segment_invalid"))?;
        if !matches!(
            name,
            "segment.json"
                | "host-service.log"
                | "host-service.log.1"
                | "events.log"
                | "events.log.1"
        ) {
            return Err(error("personal_namespace_storage_segment_unclassified"));
        }
        let m = fs::symlink_metadata(child.path())
            .map_err(|_| error("personal_namespace_storage_segment_unavailable"))?;
        if !m.is_file() || m.file_type().is_symlink() || filesystem::is_cloud_placeholder_file(&m) {
            return Err(error("personal_namespace_storage_path_denied"));
        }
    }
    let expected = selected_log_files(&p.item_kinds);
    if p.log_files
        .iter()
        .map(|f| f.file.clone())
        .collect::<BTreeSet<_>>()
        != expected
        || p.log_files.len() != expected.len()
    {
        return Err(error("personal_namespace_storage_segment_invalid"));
    }
    for file in &p.log_files {
        let metadata = match fs::symlink_metadata(segment.join(file_name(&file.file))) {
            Ok(m) => Some(m),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(error("personal_namespace_storage_segment_unavailable")),
        };
        if file.exists != metadata.is_some()
            || file.logical_bytes != metadata.as_ref().map(fs::Metadata::len)
        {
            return Err(error("personal_namespace_storage_segment_changed"));
        }
        let fields = [
            &file.device,
            &file.inode,
            &file.mode,
            &file.modified_nanoseconds,
            &file.changed_nanoseconds,
        ];
        if fields.iter().any(|v| match v {
            Some(v) => {
                !file.exists
                    || v.is_empty()
                    || v.len() > 80
                    || !v.bytes().all(|b| b.is_ascii_digit())
            }
            None => file.exists,
        }) {
            return Err(error("personal_namespace_storage_segment_invalid"));
        }
    }
    Ok(())
}
fn validate_prepare(
    root: &Path,
    id: &str,
    p: &PersonalNamespaceStoragePrepareParams,
) -> Result<(), ServiceError> {
    if p.namespace_path != root.to_string_lossy()
        || p.client_operation_id.trim().is_empty()
        || p.client_operation_id.len() > 200
        || p.item_kinds.is_empty()
        || p.item_kinds.len() > 3
        || p.item_kinds.iter().collect::<BTreeSet<_>>().len() != p.item_kinds.len()
    {
        return Err(error("personal_namespace_storage_request_invalid"));
    }
    let browser = p
        .item_kinds
        .contains(&PersonalNamespaceStorageKind::BrowserHttpCache);
    if browser != p.browser_session.is_some() {
        return Err(error("personal_namespace_storage_browser_unavailable"));
    }
    if let Some(session) = &p.browser_session {
        let expected = root
            .join("browser")
            .join("Partitions")
            .join(format!("ai-browser-{id}"));
        if session.partition != format!("persist:ai-browser-{id}")
            || session.storage_path != expected.to_string_lossy()
            || session.cache_bytes > 9_007_199_254_740_991
        {
            return Err(error("personal_namespace_storage_browser_scope_invalid"));
        }
        directory(&expected)?;
    }
    validate_segment(root, id, p)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Marker {
    schema_version: u32,
    profile_id: String,
    operation_id: String,
    namespace_identity: String,
    active: bool,
}
fn marker_path(root: &Path) -> PathBuf {
    root.join("business/personal-namespace-storage-state.json")
}
fn read_marker(root: &Path) -> Result<Option<Marker>, ServiceError> {
    let path = marker_path(root);
    match fs::symlink_metadata(&path) {
        Ok(_) => read_private(&path).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(error("personal_namespace_storage_journal_invalid")),
    }
}
fn write_marker(root: &Path, m: &Marker) -> Result<(), ServiceError> {
    let path = marker_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| error("personal_namespace_storage_namespace_invalid"))?;
    let tmp = parent.join(format!(
        "personal-namespace-storage-state.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let bytes =
        serde_json::to_vec(m).map_err(|_| error("personal_namespace_storage_journal_invalid"))?;
    let mut file = security::create_new_restricted_file(&tmp)
        .map_err(|_| error("personal_namespace_storage_journal_write_failed"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| error("personal_namespace_storage_journal_write_failed"))?;
    fs::rename(&tmp, &path)
        .map_err(|_| error("personal_namespace_storage_journal_write_failed"))?;
    filesystem::sync_directory(parent)
        .map_err(|_| error("personal_namespace_storage_journal_write_failed"))?;
    let actual: Marker = read_private(&path)?;
    if actual.profile_id != m.profile_id
        || actual.operation_id != m.operation_id
        || actual.active != m.active
        || actual.namespace_identity != m.namespace_identity
    {
        return Err(error("personal_namespace_storage_journal_write_failed"));
    }
    Ok(())
}
fn marker_for(
    root: &Path,
    op: &PersonalNamespaceStorageOperation,
    require_active: bool,
) -> Result<(), ServiceError> {
    let identity = filesystem::symlink_metadata_with_stable_identity(root)
        .map_err(|_| error("personal_namespace_storage_namespace_unavailable"))?
        .stable_identity;
    if identity != op.namespace_identity {
        return Err(error("personal_namespace_storage_namespace_changed"));
    }
    if let Some(m) = read_marker(root)? {
        if m.schema_version != 1 || m.namespace_identity != identity || !hex_id(&m.profile_id) {
            return Err(error("personal_namespace_storage_journal_invalid"));
        }
        if m.active && (m.profile_id != op.profile_id || m.operation_id != op.operation_id) {
            return Err(error("personal_namespace_storage_other_operation_active"));
        }
        if require_active
            && (!m.active || m.operation_id != op.operation_id || m.profile_id != op.profile_id)
        {
            return Err(error("personal_namespace_storage_journal_unconfirmed"));
        }
    } else if require_active {
        return Err(error("personal_namespace_storage_journal_unconfirmed"));
    }
    Ok(())
}
fn validate_started_plan(
    root: &Path,
    op: &PersonalNamespaceStorageOperation,
) -> Result<(), ServiceError> {
    validate_prepare(
        root,
        &op.profile_id,
        &PersonalNamespaceStoragePrepareParams {
            client_operation_id: "check-frozen-plan".into(),
            item_kinds: op.actions.iter().map(|a| a.kind.clone()).collect(),
            shell_owner: op.shell_owner.clone(),
            namespace_path: op.namespace_path.clone(),
            log_segment_id: op.log_segment_id.clone(),
            log_files: op.log_files.clone(),
            browser_session: op.browser_session.clone(),
        },
    )
}
fn validate_log_ack(
    root: &Path,
    op: &PersonalNamespaceStorageOperation,
    params: &Value,
) -> Result<(), ServiceError> {
    let p: contexthub_protocol::PersonalNamespaceActionAckParams = parse(params.clone())?;
    if p.outcome != contexthub_protocol::PersonalNamespaceActionOutcome::Succeeded {
        return Ok(());
    }
    let action = op
        .actions
        .iter()
        .find(|a| a.action_id == p.action_id)
        .ok_or_else(|| error("personal_namespace_storage_permit_invalid"))?;
    if action.kind == PersonalNamespaceStorageKind::BrowserHttpCache
        || action.state == PersonalNamespaceActionState::Acknowledged
    {
        return Ok(());
    }
    let segment_id = op
        .log_segment_id
        .as_ref()
        .filter(|id| hex_id(id))
        .ok_or_else(|| error("personal_namespace_storage_segment_invalid"))?;
    let segment = root.join("personal-storage-log-segments").join(segment_id);
    match fs::symlink_metadata(&segment) {
        Ok(_) => directory(&segment)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            directory(
                segment
                    .parent()
                    .ok_or_else(|| error("personal_namespace_storage_segment_invalid"))?,
            )?;
        }
        Err(_) => return Err(error("personal_namespace_storage_segment_unavailable")),
    }
    for file in selected_log_files(std::slice::from_ref(&action.kind)) {
        match fs::symlink_metadata(segment.join(file_name(&file))) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(error("personal_namespace_storage_delete_unconfirmed")),
        }
    }
    Ok(())
}
fn settle_marker(root: &Path, result: &Value) -> Result<(), ServiceError> {
    let op: PersonalNamespaceStorageOperation = parse(result.clone())?;
    if !op
        .actions
        .iter()
        .any(|a| a.state == PersonalNamespaceActionState::Started)
    {
        write_marker(
            root,
            &Marker {
                schema_version: 1,
                profile_id: op.profile_id,
                operation_id: op.operation_id,
                namespace_identity: op.namespace_identity,
                active: false,
            },
        )?;
    }
    Ok(())
}
impl AppServerServices {
    fn namespace_read(
        &mut self,
        id: &str,
        op: &str,
        instance: &str,
        now: i64,
    ) -> Result<PersonalNamespaceStorageOperation, ServiceError> {
        let v = self
            .persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .personal_namespace_storage_rpc(
                "personalNamespaceStorage/get",
                json!({"operationId":op}),
                id,
                instance,
                now,
            )
            .map_err(|e| db_error(&e))?;
        parse(v)
    }
    pub(crate) fn personal_namespace_storage_rpc(
        &mut self,
        method: &str,
        params: Value,
        instance: &str,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let (profile, id) = self.data_context()?;
        let root = namespace(&profile, &id)?;
        if method == "personalNamespaceStorage/list" {
            let _: contexthub_protocol::PersonalStorageInventoryParams = parse(params)?;
            return value(
                self.persistence
                    .as_ref()
                    .ok_or_else(|| error("persistence_unavailable"))?
                    .personal_namespace_storage_operations(&id)
                    .map_err(|e| db_error(&e))?,
            );
        }
        if matches!(
            method,
            "personalNamespaceStorage/get" | "personalNamespaceStorage/reconcile"
        ) {
            return self
                .persistence
                .as_mut()
                .ok_or_else(|| error("persistence_unavailable"))?
                .personal_namespace_storage_rpc(method, params, &id, instance, now)
                .map_err(|e| db_error(&e));
        }
        let _lock = security::try_lock_restricted_file(
            &root.join("business/personal-namespace-storage.lock"),
        )
        .map_err(|_| error("personal_namespace_storage_lock_unavailable"))?
        .ok_or_else(|| error("personal_namespace_storage_busy"))?;
        if method == "personalNamespaceStorage/prepare" {
            return self
                .namespace_prepare(&root, &id, parse(params)?, instance, now)
                .and_then(value);
        }
        let op_id = params
            .get("operationId")
            .and_then(Value::as_str)
            .ok_or_else(|| error("personal_namespace_storage_request_invalid"))?;
        let op = self.namespace_read(&id, op_id, instance, now)?;
        let start = method == "personalNamespaceStorage/start";
        let actor = matches!(
            method,
            "personalNamespaceStorage/beginAction" | "personalNamespaceStorage/ackAction"
        );
        marker_for(&root, &op, actor)?;
        if start {
            validate_started_plan(&root, &op)?;
            let p: PersonalNamespaceStorageStartParams = parse(params)?;
            self.persistence
                .as_mut()
                .ok_or_else(|| error("persistence_unavailable"))?
                .verify_personal_data_quiescence(&id, &p.operation_id, instance, &p.quiescence)
                .map_err(|e| db_error(&e))?;
            write_marker(
                &root,
                &Marker {
                    schema_version: 1,
                    profile_id: id.clone(),
                    operation_id: op.operation_id.clone(),
                    namespace_identity: op.namespace_identity.clone(),
                    active: true,
                },
            )?;
            return value(
                self.persistence
                    .as_mut()
                    .ok_or_else(|| error("persistence_unavailable"))?
                    .start_personal_namespace_storage(&p, &id, instance, now)
                    .map_err(|e| db_error(&e))?,
            );
        }
        if method == "personalNamespaceStorage/ackAction" {
            validate_log_ack(&root, &op, &params)?;
        }
        let result = self
            .persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .personal_namespace_storage_rpc(method, params, &id, instance, now)
            .map_err(|e| db_error(&e))?;
        if matches!(
            method,
            "personalNamespaceStorage/finish"
                | "personalNamespaceStorage/cancel"
                | "personalNamespaceStorage/resolveInterrupted"
        ) {
            settle_marker(&root, &result)?;
        }
        Ok(result)
    }
    fn namespace_prepare(
        &mut self,
        root: &Path,
        id: &str,
        p: PersonalNamespaceStoragePrepareParams,
        instance: &str,
        now: i64,
    ) -> Result<PersonalNamespaceStorageOperation, ServiceError> {
        validate_prepare(root, id, &p)?;
        let fingerprint = hash(&p)?;
        let db = self
            .persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?;
        if let Some(old) = db
            .personal_namespace_storage_by_client(id, &p.client_operation_id, &fingerprint)
            .map_err(|e| db_error(&e))?
        {
            return Ok(old.operation);
        }
        let namespace_identity = filesystem::symlink_metadata_with_stable_identity(root)
            .map_err(|_| error("personal_namespace_storage_namespace_unavailable"))?
            .stable_identity;
        let actions: Vec<_> = p
            .item_kinds
            .iter()
            .map(|kind| PersonalNamespaceActionReceipt {
                action_id: format!("namespace_action_{}", uuid::Uuid::new_v4().simple()),
                kind: kind.clone(),
                state: PersonalNamespaceActionState::Planned,
                outcome: None,
                reason_code: None,
                started_at: None,
                acknowledged_at: None,
                after_cache_bytes: None,
            })
            .collect();
        let preview_digest =
            hash(&json!({"profileId":id,"namespaceIdentity":namespace_identity,"request":p}))?;
        let operation = PersonalNamespaceStorageOperation {
            operation_id: format!("namespace_storage_{}", uuid::Uuid::new_v4().simple()),
            profile_id: id.into(),
            namespace_path: p.namespace_path,
            namespace_identity,
            shell_owner: p.shell_owner,
            state: PersonalStorageOperationState::Prepared,
            preview_digest,
            created_at: now,
            updated_at: now,
            log_segment_id: p.log_segment_id,
            log_files: p.log_files,
            browser_session: p.browser_session,
            actions,
            blockers: vec![],
        };
        let authorizations = operation
            .actions
            .iter()
            .map(|a| PersonalNamespaceActionAuthorization {
                action_id: a.action_id.clone(),
                permit_token: uuid::Uuid::new_v4().simple().to_string(),
                writer_lease_id: None,
                acknowledgement: None,
            })
            .collect();
        Ok(db
            .prepare_personal_namespace_storage(
                &PersonalNamespaceStorageRecord {
                    operation,
                    authorizations,
                    quiescence: None,
                    client_operation_id: p.client_operation_id,
                    request_fingerprint: fingerprint,
                },
                instance,
            )
            .map_err(|e| db_error(&e))?
            .operation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_protocol::{ModelExecutionOwner, PersonalNamespaceLogSnapshot};
    struct Fixture {
        root: PathBuf,
        namespace: PathBuf,
        profile: PathBuf,
        id: String,
        segment: String,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "cilai-namespace-classification-{}",
                uuid::Uuid::new_v4().simple()
            ));
            let namespace = root.join("cn.cilai.personal.workbench");
            let id = "11111111111111111111111111111111".to_string();
            let profile = namespace.join("business/profiles").join(&id);
            let segment = "22222222222222222222222222222222".to_string();
            security::ensure_restricted_directory(&profile).unwrap();
            let segment_root = namespace
                .join("personal-storage-log-segments")
                .join(&segment);
            security::ensure_restricted_directory(&segment_root).unwrap();
            let mut file =
                security::create_new_restricted_file(&segment_root.join("segment.json")).unwrap();
            file.write_all(
                serde_json::to_string(
                    &json!({"schemaVersion":1,"segmentId":segment,"profileId":id,"createdAt":1}),
                )
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
            file.sync_all().unwrap();
            Self {
                root,
                namespace,
                profile,
                id,
                segment,
            }
        }
        fn request(&self) -> PersonalNamespaceStoragePrepareParams {
            PersonalNamespaceStoragePrepareParams {
                client_operation_id: "test-prepare".into(),
                item_kinds: vec![PersonalNamespaceStorageKind::HostServiceLogs],
                shell_owner: ModelExecutionOwner {
                    instance_id: "classification-only".into(),
                    pid: 1,
                    start_fingerprint: "not-executed".into(),
                },
                namespace_path: self.namespace.to_str().unwrap().into(),
                log_segment_id: Some(self.segment.clone()),
                log_files: [
                    PersonalNamespaceLogFile::HostServiceCurrent,
                    PersonalNamespaceLogFile::HostServicePrevious,
                ]
                .map(|file| PersonalNamespaceLogSnapshot {
                    file,
                    exists: false,
                    device: None,
                    inode: None,
                    mode: None,
                    logical_bytes: None,
                    modified_nanoseconds: None,
                    changed_nanoseconds: None,
                })
                .to_vec(),
                browser_session: None,
            }
        }
        fn segment_root(&self) -> PathBuf {
            self.namespace
                .join("personal-storage-log-segments")
                .join(&self.segment)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
    #[test]
    fn namespace_is_derived_from_exact_personal_profile_and_does_not_accept_arbitrary_roots() {
        let f = Fixture::new();
        assert_eq!(namespace(&f.profile, &f.id).unwrap(), f.namespace);
        assert!(namespace(&f.root, &f.id).is_err());
        let mut p = f.request();
        p.namespace_path = f.root.to_str().unwrap().into();
        assert!(validate_prepare(&f.namespace, &f.id, &p).is_err());
    }
    #[test]
    fn known_cancelled_log_segment_can_be_reobserved_without_reading_body_but_unknown_content_is_protected()
     {
        let f = Fixture::new();
        let p = f.request();
        validate_prepare(&f.namespace, &f.id, &p).unwrap();
        fs::write(f.segment_root().join("preferences.json"), b"protected").unwrap();
        assert_eq!(
            validate_prepare(&f.namespace, &f.id, &p)
                .unwrap_err()
                .reason,
            "personal_namespace_storage_segment_unclassified"
        );
        assert_eq!(
            fs::read(f.segment_root().join("preferences.json")).unwrap(),
            b"protected"
        );
    }
    #[test]
    fn foreign_segment_and_changed_size_are_rejected_without_mutation() {
        let f = Fixture::new();
        let p = f.request();
        fs::write(f.segment_root().join("host-service.log"), b"retained-log").unwrap();
        assert_eq!(
            validate_prepare(&f.namespace, &f.id, &p)
                .unwrap_err()
                .reason,
            "personal_namespace_storage_segment_changed"
        );
        assert!(validate_prepare(&f.namespace, "33333333333333333333333333333333", &p).is_err());
        assert_eq!(
            fs::read(f.segment_root().join("host-service.log")).unwrap(),
            b"retained-log"
        );
    }
    #[test]
    fn uninitialized_browser_is_rejected_without_creating_a_partition() {
        let f = Fixture::new();
        let mut p = f.request();
        p.item_kinds = vec![PersonalNamespaceStorageKind::BrowserHttpCache];
        p.log_files.clear();
        p.log_segment_id = None;
        p.browser_session = Some(contexthub_protocol::PersonalNamespaceBrowserSnapshot {
            partition: format!("persist:ai-browser-{}", f.id),
            storage_path: f
                .namespace
                .join("browser/Partitions")
                .join(format!("ai-browser-{}", f.id))
                .to_str()
                .unwrap()
                .into(),
            cache_bytes: 1,
        });
        assert!(validate_prepare(&f.namespace, &f.id, &p).is_err());
        assert!(!f.namespace.join("browser").exists());
    }
    #[test]
    fn successful_log_receipt_requires_selected_files_to_be_absent_and_never_reads_them() {
        let f = Fixture::new();
        let p = f.request();
        let op = PersonalNamespaceStorageOperation {
            operation_id: "namespace_storage_test".into(),
            profile_id: f.id.clone(),
            namespace_path: p.namespace_path,
            namespace_identity: "scope-test-only".into(),
            shell_owner: p.shell_owner.clone(),
            state: PersonalStorageOperationState::Applying,
            preview_digest: "scope-test".into(),
            created_at: 1,
            updated_at: 1,
            log_segment_id: p.log_segment_id,
            log_files: p.log_files,
            browser_session: None,
            actions: vec![PersonalNamespaceActionReceipt {
                action_id: "test-action".into(),
                kind: PersonalNamespaceStorageKind::HostServiceLogs,
                state: PersonalNamespaceActionState::Started,
                outcome: None,
                reason_code: None,
                started_at: Some(1),
                acknowledged_at: None,
                after_cache_bytes: None,
            }],
            blockers: vec![],
        };
        let ack = json!({"operationId":op.operation_id,"actionId":"test-action","permitToken":"scope-test","shellOwner":op.shell_owner,"outcome":"succeeded","reasonCode":null,"afterCacheBytes":null});
        let path = f.segment_root().join("host-service.log");
        fs::write(&path, b"preserved-log").unwrap();
        assert_eq!(
            validate_log_ack(&f.namespace, &op, &ack)
                .unwrap_err()
                .reason,
            "personal_namespace_storage_delete_unconfirmed"
        );
        assert_eq!(fs::read(&path).unwrap(), b"preserved-log");
        fs::remove_file(path).unwrap();
        validate_log_ack(&f.namespace, &op, &ack).unwrap();
    }
    #[test]
    fn shared_namespace_marker_survives_reopen_and_refuses_other_profile_until_explicitly_settled()
    {
        let f = Fixture::new();
        let identity = filesystem::symlink_metadata_with_stable_identity(&f.namespace)
            .unwrap()
            .stable_identity;
        let mut marker = Marker {
            schema_version: 1,
            profile_id: f.id.clone(),
            operation_id: "namespace_storage_one".into(),
            namespace_identity: identity.clone(),
            active: true,
        };
        write_marker(&f.namespace, &marker).unwrap();
        let mut op = PersonalNamespaceStorageOperation {
            operation_id: "namespace_storage_two".into(),
            profile_id: "33333333333333333333333333333333".into(),
            namespace_path: f.namespace.to_str().unwrap().into(),
            namespace_identity: identity,
            shell_owner: f.request().shell_owner,
            state: PersonalStorageOperationState::Prepared,
            preview_digest: "test-only".into(),
            created_at: 1,
            updated_at: 1,
            log_segment_id: None,
            log_files: vec![],
            browser_session: None,
            actions: vec![],
            blockers: vec![],
        };
        assert_eq!(
            marker_for(&f.namespace, &op, false).unwrap_err().reason,
            "personal_namespace_storage_other_operation_active"
        );
        op.profile_id = f.id.clone();
        op.operation_id = marker.operation_id.clone();
        marker_for(&f.namespace, &op, true).unwrap();
        marker.active = false;
        write_marker(&f.namespace, &marker).unwrap();
        assert!(marker_for(&f.namespace, &op, true).is_err());
        marker_for(&f.namespace, &op, false).unwrap();
    }
}
