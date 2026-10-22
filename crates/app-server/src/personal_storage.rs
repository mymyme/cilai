//! 只处理当前个人资料的受管清理；资源仍被引用、路径未知或结果未明时保留。
use crate::{AppServerServices, ServiceError};
use contexthub_persistence_sqlite::{PersonalStoragePathReference, PersonalStorageRecord};
use contexthub_platform_adapter::{filesystem, security};
use contexthub_protocol::{
    PersonalStorageCategory, PersonalStorageConfirmParams, PersonalStorageCoverage,
    PersonalStorageCoverageState, PersonalStorageExecuteParams, PersonalStorageInventory,
    PersonalStorageInventoryParams, PersonalStorageItem, PersonalStorageItemResult,
    PersonalStorageOperation, PersonalStorageOperationState, PersonalStorageOutcome,
    PersonalStorageParams, PersonalStoragePrepareParams, PersonalStorageResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};
const MAX_FILES: usize = 100_000;
const MAX_SELECTED: usize = 500;
fn error(reason: &'static str) -> ServiceError {
    ServiceError::new(reason)
}
fn db_error(e: &contexthub_persistence_sqlite::PersistenceError) -> ServiceError {
    error(e.reason().unwrap_or("personal_storage_storage_failed"))
}
fn bytes(value: &impl Serialize) -> Result<Vec<u8>, ServiceError> {
    serde_json::to_vec(value).map_err(|_| error("personal_storage_record_invalid"))
}
fn hash(value: &impl Serialize) -> Result<String, ServiceError> {
    Ok(format!("sha256:{:x}", Sha256::digest(bytes(value)?)))
}
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ServiceError> {
    serde_json::from_value(value).map_err(|_| error("personal_storage_request_invalid"))
}
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0', ':'])
        && path
            .split('/')
            .all(|p| !p.is_empty() && !matches!(p, "." | ".."))
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn identifier(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn metadata(path: &Path) -> Result<filesystem::FileMetadataSnapshot, ServiceError> {
    let snapshot = filesystem::symlink_metadata_with_stable_identity(path)
        .map_err(|_| error("personal_storage_content_unavailable"))?;
    if snapshot.metadata.file_type().is_symlink()
        || filesystem::is_cloud_placeholder_file(&snapshot.metadata)
    {
        return Err(error("personal_storage_path_not_managed"));
    }
    Ok(snapshot)
}
fn directory(path: &Path) -> Result<(), ServiceError> {
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor)
            .map_err(|_| error("personal_storage_content_unavailable"))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(error("personal_storage_path_not_managed"));
        }
    }
    Ok(())
}
fn children(path: &Path) -> Result<Vec<(String, PathBuf)>, ServiceError> {
    directory(path)?;
    let mut out = vec![];
    for entry in fs::read_dir(path).map_err(|_| error("personal_storage_content_unavailable"))? {
        let entry = entry.map_err(|_| error("personal_storage_content_unavailable"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| error("personal_storage_path_not_managed"))?;
        out.push((name, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Node {
    relative_path: String,
    identity: String,
    directory: bool,
    bytes: u64,
    modified: String,
    digest: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Candidate {
    item_id: String,
    relative_path: String,
    nodes: Vec<Node>,
    snapshot_digest: String,
}
struct Definition {
    path: String,
    category: PersonalStorageCategory,
    eligible: bool,
    reason: &'static str,
}
struct Scan {
    inventory: PersonalStorageInventory,
    candidates: Vec<Candidate>,
}
fn content_hash(
    path: &Path,
    expected: &filesystem::FileMetadataSnapshot,
) -> Result<String, ServiceError> {
    let mut file = filesystem::open_regular_file_no_follow(path, false)
        .map_err(|_| error("personal_storage_content_unavailable"))?;
    let opened = filesystem::opened_file_metadata_with_stable_identity(&file)
        .map_err(|_| error("personal_storage_content_unavailable"))?;
    if opened.stable_identity != expected.stable_identity {
        return Err(error("personal_storage_content_changed"));
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 65536];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| error("personal_storage_content_unavailable"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let after = metadata(path)?;
    if after.stable_identity != expected.stable_identity
        || after.metadata.len() != expected.metadata.len()
        || after.metadata.modified().ok() != expected.metadata.modified().ok()
    {
        return Err(error("personal_storage_content_changed"));
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}
fn tree(
    path: &Path,
    relative: &str,
    hash_contents: bool,
    nodes: &mut Vec<Node>,
) -> Result<(), ServiceError> {
    if nodes.len() >= MAX_FILES {
        return Err(error("personal_storage_scan_limit"));
    }
    let m = metadata(path)?;
    if !m.metadata.is_file() && !m.metadata.is_dir() {
        return Err(error("personal_storage_path_not_managed"));
    }
    let modified = m
        .metadata
        .modified()
        .map_err(|_| error("personal_storage_content_unavailable"))?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error("personal_storage_content_unavailable"))?
        .as_nanos()
        .to_string();
    let digest = if hash_contents && m.metadata.is_file() {
        Some(content_hash(path, &m)?)
    } else {
        None
    };
    nodes.push(Node {
        relative_path: relative.into(),
        identity: m.stable_identity,
        directory: m.metadata.is_dir(),
        bytes: if m.metadata.is_file() {
            m.metadata.len()
        } else {
            0
        },
        modified,
        digest,
    });
    if m.metadata.is_dir() {
        for (name, child) in children(path)? {
            tree(
                &child,
                &if relative.is_empty() {
                    name
                } else {
                    format!("{relative}/{name}")
                },
                hash_contents,
                nodes,
            )?;
        }
    }
    Ok(())
}
fn snapshot(path: &Path, hash_contents: bool) -> Result<Vec<Node>, ServiceError> {
    if let Some(parent) = path.parent() {
        directory(parent)?;
    }
    let mut nodes = vec![];
    tree(path, "", hash_contents, &mut nodes)?;
    nodes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(nodes)
}
fn item_id(profile: &str, path: &str) -> Result<String, ServiceError> {
    Ok(format!(
        "storage_item_{}",
        hash(&json!({"profileId":profile,"path":path}))?.trim_start_matches("sha256:")
    ))
}
fn referenced(
    path: &str,
    refs: &[PersonalStoragePathReference],
) -> Vec<contexthub_protocol::PersonalStorageReference> {
    let mut seen = BTreeSet::new();
    refs.iter()
        .filter(|r| {
            r.relative_path.is_empty()
                || path == r.relative_path
                || r.relative_path.starts_with(&format!("{path}/"))
                || path.starts_with(&format!("{}/", r.relative_path))
        })
        .filter_map(|r| {
            if seen.insert((r.reference.kind.clone(), r.reference.object_id.clone())) {
                Some(r.reference.clone())
            } else {
                None
            }
        })
        .collect()
}
fn definition(
    path: &str,
    category: PersonalStorageCategory,
    eligible: bool,
    reason: &'static str,
) -> Definition {
    Definition {
        path: path.into(),
        category,
        eligible,
        reason,
    }
}
fn definitions(profile: &Path) -> Result<Vec<Definition>, ServiceError> {
    let mut out = vec![];
    for (name, _) in children(profile)? {
        match name.as_str() {
            "ai-runtime" => runtime_definitions(profile, &mut out)?,
            "skills" => skill_definitions(profile, &mut out)?,
            "personal-package-imports" => {
                for (name, _) in children(&profile.join("personal-package-imports"))? {
                    let allowed = name.strip_prefix("import_").is_some_and(identifier);
                    out.push(definition(
                        &format!("personal-package-imports/{name}"),
                        PersonalStorageCategory::Cache,
                        allowed,
                        "personal_storage_import_candidate",
                    ));
                }
            }
            "contexthub.db" | "contexthub.db-wal" | "contexthub.db-shm" => out.push(definition(
                &name,
                PersonalStorageCategory::BusinessDatabase,
                false,
                "personal_storage_business_database_protected",
            )),
            "personal-capabilities" | "capability-projects" => out.push(definition(
                &name,
                PersonalStorageCategory::Applications,
                false,
                "personal_storage_application_source_protected",
            )),
            "personal-capability-data"
            | "personal-capability-preview-data"
            | "personal-capability-slots" => {
                out.push(definition(
                    &name,
                    PersonalStorageCategory::ApplicationData,
                    false,
                    "personal_storage_application_data_protected",
                ));
            }
            "attachments" | "downloads" => out.push(definition(
                &name,
                PersonalStorageCategory::Attachments,
                false,
                "personal_storage_user_material_protected",
            )),
            "ai-history-backup"
            | "schema-backups"
            | "personal-data-operations"
            | "personal-storage-operations" => out.push(definition(
                &name,
                PersonalStorageCategory::RecoveryCopies,
                false,
                "personal_storage_recovery_copy_protected",
            )),
            "mcp-runtime" => out.push(definition(
                &name,
                PersonalStorageCategory::Configuration,
                false,
                "personal_storage_mcp_private_home_protected",
            )),
            "model-credential-operation.lock" => out.push(definition(
                &name,
                PersonalStorageCategory::Configuration,
                false,
                "personal_storage_credential_lock_protected",
            )),
            "run" => out.push(definition(
                &name,
                PersonalStorageCategory::Configuration,
                false,
                "personal_storage_instance_state_protected",
            )),
            _ => out.push(definition(
                &name,
                PersonalStorageCategory::OtherProtected,
                false,
                "personal_storage_unclassified_protected",
            )),
        }
    }
    Ok(out)
}
fn skill_definitions(profile: &Path, out: &mut Vec<Definition>) -> Result<(), ServiceError> {
    for (name, _) in children(&profile.join("skills"))? {
        if name != "user-packages" {
            out.push(definition(
                &format!("skills/{name}"),
                PersonalStorageCategory::Skills,
                false,
                "personal_storage_user_skill_protected",
            ));
            continue;
        }
        for (skill, _) in children(&profile.join("skills/user-packages"))? {
            let parent = format!("skills/user-packages/{skill}");
            match children(&profile.join(&parent)) {
                Ok(versions) => {
                    for (version, _) in versions {
                        out.push(definition(
                            &format!("{parent}/{version}"),
                            PersonalStorageCategory::Skills,
                            identifier(&version),
                            "personal_storage_unreferenced_skill_package",
                        ));
                    }
                }
                Err(_) => out.push(definition(
                    &parent,
                    PersonalStorageCategory::Skills,
                    false,
                    "personal_storage_unclassified_protected",
                )),
            }
        }
    }
    Ok(())
}
fn runtime_definitions(profile: &Path, out: &mut Vec<Definition>) -> Result<(), ServiceError> {
    for (name, _) in children(&profile.join("ai-runtime"))? {
        let path = format!("ai-runtime/{name}");
        match name.as_str() {
            "attachments" | "runs" | "artifact-versions" | "conversation-workspaces" => {
                out.push(definition(
                    &path,
                    PersonalStorageCategory::Attachments,
                    false,
                    "personal_storage_user_material_protected",
                ));
            }
            "history-migration-report.json" => out.push(definition(
                &path,
                PersonalStorageCategory::RecoveryCopies,
                false,
                "personal_storage_recovery_copy_protected",
            )),
            "mcp-runtime" => out.push(definition(
                &path,
                PersonalStorageCategory::Configuration,
                false,
                "personal_storage_mcp_private_home_protected",
            )),
            "memory.lock" => out.push(definition(
                &path,
                PersonalStorageCategory::Configuration,
                false,
                "personal_storage_instance_state_protected",
            )),
            "skill-snapshots" => out.push(definition(
                &path,
                PersonalStorageCategory::Cache,
                true,
                "personal_storage_regenerable_skill_snapshot",
            )),
            "cilai-harness" | "cilai-application-harness" => {
                wrapper_definitions(profile, &path, out)?;
            }
            "model-connections" => {
                for (scope, _) in children(&profile.join(&path))? {
                    let scope_path = format!("{path}/{scope}");
                    if scope.len() != 64 || !scope.bytes().all(|b| b.is_ascii_hexdigit()) {
                        out.push(definition(
                            &scope_path,
                            PersonalStorageCategory::OtherProtected,
                            false,
                            "personal_storage_unclassified_protected",
                        ));
                        continue;
                    }
                    for (kind, _) in children(&profile.join(&scope_path))? {
                        let wrapper = format!("{scope_path}/{kind}");
                        if matches!(kind.as_str(), "workbench" | "application") {
                            wrapper_definitions(profile, &wrapper, out)?;
                        } else {
                            out.push(definition(
                                &wrapper,
                                PersonalStorageCategory::OtherProtected,
                                false,
                                "personal_storage_unclassified_protected",
                            ));
                        }
                    }
                }
            }
            _ => out.push(definition(
                &path,
                PersonalStorageCategory::Configuration,
                false,
                "personal_storage_runtime_configuration_protected",
            )),
        }
    }
    Ok(())
}
fn wrapper_definitions(
    profile: &Path,
    wrapper: &str,
    out: &mut Vec<Definition>,
) -> Result<(), ServiceError> {
    for (name, _) in children(&profile.join(wrapper))? {
        let path = format!("{wrapper}/{name}");
        if name != "official-runtime" {
            out.push(definition(
                &path,
                PersonalStorageCategory::Configuration,
                false,
                "personal_storage_runtime_configuration_protected",
            ));
            continue;
        }
        for (name, _) in children(&profile.join(&path))? {
            let entry = format!("{path}/{name}");
            match name.as_str() {
                "shell_snapshots" => out.push(definition(
                    &entry,
                    PersonalStorageCategory::Cache,
                    true,
                    "personal_storage_runtime_shell_cache",
                )),
                "log" | "logs" => out.push(definition(
                    &entry,
                    PersonalStorageCategory::Logs,
                    true,
                    "personal_storage_runtime_log",
                )),
                "process-home" => {
                    for (name, _) in children(&profile.join(&entry))? {
                        let child = format!("{entry}/{name}");
                        out.push(if name == "cache" {
                            definition(
                                &child,
                                PersonalStorageCategory::Cache,
                                true,
                                "personal_storage_runtime_cache",
                            )
                        } else {
                            definition(
                                &child,
                                PersonalStorageCategory::Configuration,
                                false,
                                "personal_storage_runtime_configuration_protected",
                            )
                        });
                    }
                }
                "memories"
                | "memories_v2"
                | "memories_extensions"
                | "memory"
                | "memories_1.sqlite"
                | "memories_1.sqlite-wal"
                | "memories_1.sqlite-shm" => out.push(definition(
                    &entry,
                    PersonalStorageCategory::NativeMemory,
                    false,
                    "personal_storage_native_memory_protected",
                )),
                "sessions"
                | "archived_sessions"
                | "history.jsonl"
                | "session_index.jsonl"
                | "state_5.sqlite"
                | "state_5.sqlite-wal"
                | "state_5.sqlite-shm"
                | "goals_1.sqlite"
                | "goals_1.sqlite-wal"
                | "goals_1.sqlite-shm"
                | "queue_1.sqlite"
                | "queue_1.sqlite-wal"
                | "queue_1.sqlite-shm" => out.push(definition(
                    &entry,
                    PersonalStorageCategory::NativeHistory,
                    false,
                    "personal_storage_native_history_protected",
                )),
                "logs_2.sqlite" | "logs_2.sqlite-wal" | "logs_2.sqlite-shm" => {
                    out.push(definition(
                        &entry,
                        PersonalStorageCategory::Logs,
                        false,
                        "personal_storage_native_log_database_protected",
                    ));
                }
                _ => out.push(definition(
                    &entry,
                    PersonalStorageCategory::Configuration,
                    false,
                    "personal_storage_runtime_configuration_protected",
                )),
            }
        }
    }
    Ok(())
}
fn measure_definition(
    profile: &Path,
    id: &str,
    refs: &[PersonalStoragePathReference],
    definition: Definition,
    inventory: &mut PersonalStorageInventory,
    candidates: &mut Vec<Candidate>,
) -> Result<(), ServiceError> {
    let resource_refs = referenced(&definition.path, refs);
    let eligible = definition.eligible && resource_refs.is_empty();
    let mut reasons = if resource_refs.is_empty() {
        vec![definition.reason.into()]
    } else {
        vec!["personal_storage_resource_referenced".into()]
    };
    let measured = snapshot(&profile.join(&definition.path), eligible);
    let (mut logical_bytes, mut file_count, mut snapshot_digest) = (None, None, None);
    let mut allowed = eligible;
    match measured {
        Ok(nodes) => {
            let size = nodes
                .iter()
                .try_fold(0_u64, |a, n| a.checked_add(n.bytes))
                .ok_or_else(|| error("personal_storage_scan_limit"))?;
            let count = u32::try_from(nodes.iter().filter(|n| !n.directory).count())
                .map_err(|_| error("personal_storage_scan_limit"))?;
            let digest = hash(&nodes)?;
            logical_bytes = Some(size);
            file_count = Some(count);
            snapshot_digest = Some(digest.clone());
            inventory.known_logical_bytes = inventory
                .known_logical_bytes
                .checked_add(size)
                .ok_or_else(|| error("personal_storage_scan_limit"))?;
            if eligible {
                candidates.push(Candidate {
                    item_id: item_id(id, &definition.path)?,
                    relative_path: definition.path.clone(),
                    nodes,
                    snapshot_digest: digest,
                });
            }
        }
        Err(reason) => {
            allowed = false;
            reasons.push(reason.reason.into());
            inventory.size_complete = false;
            inventory.issues.push(reason.reason.into());
        }
    }
    inventory.items.push(PersonalStorageItem {
        item_id: item_id(id, &definition.path)?,
        category: definition.category,
        display_path: profile
            .join(&definition.path)
            .to_string_lossy()
            .into_owned(),
        managed_relative_path: Some(definition.path),
        logical_bytes,
        file_count,
        snapshot_digest,
        cleanup_allowed: allowed,
        reasons,
        references: resource_refs,
    });

    Ok(())
}
impl AppServerServices {
    fn storage_scan(&self, now: i64) -> Result<Scan, ServiceError> {
        let (profile, id) = self.data_context()?;
        let db = self
            .persistence
            .as_ref()
            .ok_or_else(|| error("persistence_unavailable"))?;
        let refs = db
            .personal_storage_references(&profile)
            .map_err(|e| db_error(&e))?;
        let mut inventory = PersonalStorageInventory {
            profile_id: id.clone(),
            profile_path: profile.to_string_lossy().into_owned(),
            scanned_at: now,
            known_logical_bytes: 0,
            size_complete: true,
            active_writer_count: db
                .personal_storage_active_writer_count(&id)
                .map_err(|e| db_error(&e))?,
            coverage: vec![
                PersonalStorageCoverage {
                    category: PersonalStorageCategory::OtherProtected,
                    scope: "current_profile".into(),
                    state: PersonalStorageCoverageState::Covered,
                    reason: None,
                },
                PersonalStorageCoverage {
                    category: PersonalStorageCategory::Logs,
                    scope: "application_namespace_logs".into(),
                    state: PersonalStorageCoverageState::NotScanned,
                    reason: Some("personal_storage_host_namespace_adapter_required".into()),
                },
                PersonalStorageCoverage {
                    category: PersonalStorageCategory::Cache,
                    scope: "application_namespace_browser_cache".into(),
                    state: PersonalStorageCoverageState::NotScanned,
                    reason: Some("personal_storage_browser_session_excluded".into()),
                },
            ],
            items: vec![],
            issues: vec![],
        };
        let mut candidates = vec![];
        for definition in definitions(&profile)? {
            measure_definition(
                &profile,
                &id,
                &refs,
                definition,
                &mut inventory,
                &mut candidates,
            )?;
        }
        for workspace in db
            .personal_storage_workspace_references()
            .map_err(|e| db_error(&e))?
        {
            inventory.items.push(PersonalStorageItem {
                item_id: item_id(
                    &id,
                    &format!("external-workspace/{}", workspace.workspace_id),
                )?,
                category: PersonalStorageCategory::ProjectOriginals,
                display_path: workspace.path,
                managed_relative_path: None,
                logical_bytes: None,
                file_count: None,
                snapshot_digest: None,
                cleanup_allowed: false,
                reasons: vec!["personal_storage_project_originals_not_scanned".into()],
                references: vec![contexthub_protocol::PersonalStorageReference {
                    kind: "workspace".into(),
                    object_id: workspace.workspace_id,
                }],
            });
            inventory.coverage.push(PersonalStorageCoverage {
                category: PersonalStorageCategory::ProjectOriginals,
                scope: workspace.display_name,
                state: PersonalStorageCoverageState::ExternalReference,
                reason: Some("personal_storage_project_originals_not_scanned".into()),
            });
        }
        inventory.size_complete = false;
        inventory.issues.sort();
        inventory.issues.dedup();
        inventory
            .items
            .sort_by(|a, b| a.display_path.cmp(&b.display_path));
        Ok(Scan {
            inventory,
            candidates,
        })
    }
    fn storage_read(&self, profile: &str, id: &str) -> Result<PersonalStorageRecord, ServiceError> {
        self.persistence
            .as_ref()
            .ok_or_else(|| error("persistence_unavailable"))?
            .personal_storage_operation(profile, id)
            .map_err(|e| db_error(&e))?
            .ok_or_else(|| error("personal_storage_operation_not_found"))
    }
    fn storage_transition(
        &mut self,
        op: &PersonalStorageOperation,
        expected: &PersonalStorageOperationState,
    ) -> Result<PersonalStorageOperation, ServiceError> {
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .transition_personal_storage(op, expected)
            .map_err(|e| db_error(&e))
    }
    pub(crate) fn personal_storage_rpc(
        &mut self,
        method: &str,
        params: Value,
        instance: &str,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let (profile, id) = self.data_context()?;
        let op = match method {
            "personalStorage/inventory" => {
                let _: PersonalStorageInventoryParams = parse(params)?;
                return serde_json::to_value(self.storage_scan(now)?.inventory)
                    .map_err(|_| error("personal_storage_record_invalid"));
            }
            "personalStorage/prepare" => self.storage_prepare(parse(params)?, now)?,
            "personalStorage/execute" => {
                self.storage_execute(&profile, &id, &parse(params)?, instance, now)?
            }
            "personalStorage/get" | "personalStorage/reconcile" => {
                let p: PersonalStorageParams = parse(params)?;
                self.storage_read(&id, &p.operation_id)?.operation
            }
            "personalStorage/list" => {
                let _: PersonalStorageInventoryParams = parse(params)?;
                let ops = self
                    .persistence
                    .as_ref()
                    .ok_or_else(|| error("persistence_unavailable"))?
                    .personal_storage_operations(&id)
                    .map_err(|e| db_error(&e))?;
                return serde_json::to_value(
                    ops.into_iter().map(|r| r.operation).collect::<Vec<_>>(),
                )
                .map_err(|_| error("personal_storage_record_invalid"));
            }
            "personalStorage/cancel" => {
                let p: PersonalStorageConfirmParams = parse(params)?;
                let mut op = self.storage_read(&id, &p.operation_id)?.operation;
                if op.preview_digest != p.preview_digest {
                    return Err(error("personal_storage_preview_changed"));
                }
                if op.state == PersonalStorageOperationState::Cancelled {
                    return serde_json::to_value(op)
                        .map_err(|_| error("personal_storage_record_invalid"));
                }
                if !matches!(
                    op.state,
                    PersonalStorageOperationState::Prepared
                        | PersonalStorageOperationState::Incomplete
                ) {
                    return Err(error("personal_storage_operation_unconfirmed"));
                }
                let old = op.state.clone();
                op.state = PersonalStorageOperationState::Cancelled;
                op.updated_at = now;
                self.storage_transition(&op, &old)?
            }
            "personalStorage/resolveInterrupted" => {
                self.storage_resolve_interrupted(&profile, &id, &parse(params)?, now)?
            }
            _ => return Err(error("personal_storage_method_unknown")),
        };
        serde_json::to_value(op).map_err(|_| error("personal_storage_record_invalid"))
    }
    fn storage_prepare(
        &mut self,
        p: PersonalStoragePrepareParams,
        now: i64,
    ) -> Result<PersonalStorageOperation, ServiceError> {
        if p.item_ids.is_empty()
            || p.item_ids.len() > MAX_SELECTED
            || p.client_operation_id.trim().is_empty()
            || p.client_operation_id.len() > 240
            || p.item_ids.iter().collect::<BTreeSet<_>>().len() != p.item_ids.len()
        {
            return Err(error("personal_storage_request_invalid"));
        }
        let (_, id) = self.data_context()?;
        let fingerprint = hash(&p)?;
        if let Some(old) = self
            .persistence
            .as_ref()
            .ok_or_else(|| error("persistence_unavailable"))?
            .personal_storage_by_client(&id, &p.client_operation_id, &fingerprint)
            .map_err(|e| db_error(&e))?
        {
            return Ok(old.operation);
        }
        let scan = self.storage_scan(now)?;
        let mut items = vec![];
        let mut candidates = vec![];
        let mut blockers = vec![];
        let mut size = 0_u64;
        for selected in &p.item_ids {
            let item = scan
                .inventory
                .items
                .iter()
                .find(|item| &item.item_id == selected)
                .ok_or_else(|| error("personal_storage_item_unknown"))?;
            if !item.cleanup_allowed {
                blockers.push("personal_storage_selected_item_protected".into());
            }
            if let Some(bytes) = item.logical_bytes {
                size = size
                    .checked_add(bytes)
                    .ok_or_else(|| error("personal_storage_scan_limit"))?;
            }
            if let Some(candidate) = scan.candidates.iter().find(|c| &c.item_id == selected) {
                candidates.push(candidate.clone());
            }
            items.push(item.clone());
        }
        items.sort_by(|a, b| a.item_id.cmp(&b.item_id));
        candidates.sort_by(|a, b| a.item_id.cmp(&b.item_id));
        blockers.sort();
        blockers.dedup();
        let digest = hash(&json!({"profileId":id,"items":items,"candidates":candidates}))?;
        let op = PersonalStorageOperation {
            operation_id: format!("storage_{}", uuid::Uuid::new_v4().simple()),
            profile_id: id,
            state: PersonalStorageOperationState::Prepared,
            preview_digest: digest,
            created_at: now,
            updated_at: now,
            selected_items: items,
            selected_logical_bytes: size,
            requires_quiescence: true,
            blockers,
            result: None,
        };
        let record = PersonalStorageRecord {
            operation: op,
            candidates: serde_json::to_value(candidates)
                .map_err(|_| error("personal_storage_record_invalid"))?,
            client_operation_id: p.client_operation_id,
            request_fingerprint: fingerprint,
        };
        Ok(self
            .persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .prepare_personal_storage(&record)
            .map_err(|e| db_error(&e))?
            .operation)
    }
    fn storage_fail_before_apply(
        &mut self,
        mut op: PersonalStorageOperation,
        reason: &str,
        now: i64,
    ) -> Result<PersonalStorageOperation, ServiceError> {
        op.state = PersonalStorageOperationState::Incomplete;
        op.updated_at = now;
        op.blockers = vec![reason.into()];
        op.result = Some(results(
            op.selected_items
                .iter()
                .map(|item| retained(item, Some(reason)))
                .collect(),
        ));
        self.storage_transition(&op, &PersonalStorageOperationState::Prepared)
    }
    fn storage_execute(
        &mut self,
        profile: &Path,
        id: &str,
        p: &PersonalStorageExecuteParams,
        instance: &str,
        now: i64,
    ) -> Result<PersonalStorageOperation, ServiceError> {
        let record = self.storage_read(id, &p.operation_id)?;
        let mut op = record.operation;
        if op.preview_digest != p.preview_digest {
            return Err(error("personal_storage_preview_changed"));
        }
        if p.client_operation_id.trim().is_empty() || p.client_operation_id.len() > 240 {
            return Err(error("personal_storage_request_invalid"));
        }
        if op.state == PersonalStorageOperationState::Completed {
            return Ok(op);
        }
        if op.state != PersonalStorageOperationState::Prepared {
            return Err(error("personal_storage_operation_unconfirmed"));
        }
        let verified = match &p.quiescence {
            Some(proof) => self
                .persistence
                .as_mut()
                .ok_or_else(|| error("persistence_unavailable"))?
                .verify_personal_data_quiescence(id, &op.operation_id, instance, proof)
                .map_err(|e| db_error(&e)),
            None => Err(error("personal_data_quiescence_unconfirmed")),
        };
        if let Err(reason) = verified {
            return self.storage_fail_before_apply(op, reason.reason, now);
        }
        if !op.blockers.is_empty() {
            return self.storage_fail_before_apply(
                op,
                "personal_storage_selected_item_protected",
                now,
            );
        }
        let candidates: Vec<Candidate> = serde_json::from_value(record.candidates)
            .map_err(|_| error("personal_storage_record_invalid"))?;
        let current = match self.storage_scan(now) {
            Ok(scan) => scan,
            Err(reason) => return self.storage_fail_before_apply(op, reason.reason, now),
        };
        if candidates.len() != op.selected_items.len()
            || candidates.iter().any(|candidate| {
                current
                    .candidates
                    .iter()
                    .find(|c| c.item_id == candidate.item_id)
                    != Some(candidate)
            })
        {
            return self.storage_fail_before_apply(op, "personal_storage_content_changed", now);
        }
        let staging = staging_root(profile, &op.operation_id)?;
        restricted_directory(&staging)?;
        op.state = PersonalStorageOperationState::Applying;
        op.updated_at = now;
        op.result = Some(results(
            op.selected_items
                .iter()
                .map(|item| retained(item, Some("personal_storage_not_processed")))
                .collect(),
        ));
        op = self.storage_transition(&op, &PersonalStorageOperationState::Prepared)?;
        op = self.storage_apply_candidates(profile, &staging, &candidates, op, now)?;
        let complete = op.result.as_ref().is_some_and(|r| {
            r.items
                .iter()
                .all(|i| i.outcome == PersonalStorageOutcome::Deleted)
        });
        op.state = if complete {
            PersonalStorageOperationState::Completed
        } else {
            PersonalStorageOperationState::Incomplete
        };
        op.updated_at = now;
        op.blockers = op
            .result
            .as_ref()
            .map(|r| r.issues.clone())
            .unwrap_or_default();
        self.storage_transition(&op, &PersonalStorageOperationState::Applying)
    }
    fn storage_apply_candidates(
        &mut self,
        profile: &Path,
        staging: &Path,
        candidates: &[Candidate],
        mut op: PersonalStorageOperation,
        now: i64,
    ) -> Result<PersonalStorageOperation, ServiceError> {
        for candidate in candidates {
            let outcome = delete_candidate(profile, staging, candidate);
            let mut progress = op
                .result
                .take()
                .ok_or_else(|| error("personal_storage_record_invalid"))?
                .items;
            let position = progress
                .iter()
                .position(|r| r.item_id == candidate.item_id)
                .ok_or_else(|| error("personal_storage_record_invalid"))?;
            progress[position] = outcome;
            op.result = Some(results(progress));
            op.updated_at = now;
            op = self.storage_transition(&op, &PersonalStorageOperationState::Applying)?;
            if op
                .result
                .as_ref()
                .is_some_and(|r| r.items[position].outcome != PersonalStorageOutcome::Deleted)
            {
                break;
            }
        }
        Ok(op)
    }
    fn storage_resolve_interrupted(
        &mut self,
        profile: &Path,
        id: &str,
        p: &PersonalStorageConfirmParams,
        now: i64,
    ) -> Result<PersonalStorageOperation, ServiceError> {
        let record = self.storage_read(id, &p.operation_id)?;
        let mut op = record.operation;
        if op.preview_digest != p.preview_digest {
            return Err(error("personal_storage_preview_changed"));
        }
        if op.state != PersonalStorageOperationState::Applying {
            return Ok(op);
        }
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .verify_interrupted_personal_data(id, &op.operation_id)
            .map_err(|e| db_error(&e))?;
        let candidates: Vec<Candidate> = serde_json::from_value(record.candidates)
            .map_err(|_| error("personal_storage_record_invalid"))?;
        let staging = staging_root(profile, &op.operation_id)?;
        if candidates.iter().any(|candidate| {
            !safe_relative(&candidate.relative_path)
                || !candidate
                    .item_id
                    .strip_prefix("storage_item_")
                    .is_some_and(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        }) {
            return Err(error("personal_storage_record_invalid"));
        }
        let old = op.result.take();
        let items = candidates
            .iter()
            .map(|candidate| {
                reconcile_item(profile, &staging, candidate, old.as_ref(), &op.operation_id)
            })
            .collect::<Vec<_>>();
        let complete = items
            .iter()
            .all(|r| r.outcome == PersonalStorageOutcome::Deleted);
        op.result = Some(results(items));
        op.state = if complete {
            PersonalStorageOperationState::Completed
        } else {
            PersonalStorageOperationState::Incomplete
        };
        op.updated_at = now;
        op.blockers = op
            .result
            .as_ref()
            .map(|r| r.issues.clone())
            .unwrap_or_default();
        self.storage_transition(&op, &PersonalStorageOperationState::Applying)
    }
}
fn reconcile_item(
    profile: &Path,
    staging: &Path,
    candidate: &Candidate,
    old: Option<&PersonalStorageResult>,
    operation: &str,
) -> PersonalStorageItemResult {
    let source = profile.join(&candidate.relative_path);
    let target = staging.join(&candidate.item_id);
    let source_exists = fs::symlink_metadata(&source);
    let target_exists = fs::symlink_metadata(&target);
    if missing(&source_exists) && missing(&target_exists) {
        if let Some(item) = old.and_then(|r| {
            r.items.iter().find(|i| {
                i.item_id == candidate.item_id && i.outcome == PersonalStorageOutcome::Deleted
            })
        }) {
            return item.clone();
        }
        return uncertain(candidate, None, "personal_storage_outcome_unconfirmed");
    }
    if target_exists.is_ok() {
        return uncertain(
            candidate,
            Some(relative_staging(operation, &candidate.item_id)),
            "personal_storage_interrupted_content_retained",
        );
    }
    if missing(&target_exists)
        && snapshot(&source, true).is_ok_and(|nodes| nodes == candidate.nodes)
    {
        return PersonalStorageItemResult {
            item_id: candidate.item_id.clone(),
            outcome: PersonalStorageOutcome::Retained,
            deleted_logical_bytes: Some(0),
            deleted_file_count: Some(0),
            retained_relative_path: Some(candidate.relative_path.clone()),
            reason: Some("personal_storage_interrupted_content_retained".into()),
        };
    }
    uncertain(candidate, None, "personal_storage_outcome_unconfirmed")
}
fn retained(item: &PersonalStorageItem, reason: Option<&str>) -> PersonalStorageItemResult {
    PersonalStorageItemResult {
        item_id: item.item_id.clone(),
        outcome: PersonalStorageOutcome::Retained,
        deleted_logical_bytes: Some(0),
        deleted_file_count: Some(0),
        retained_relative_path: item.managed_relative_path.clone(),
        reason: reason.map(String::from),
    }
}
fn uncertain(
    candidate: &Candidate,
    path: Option<String>,
    reason: &str,
) -> PersonalStorageItemResult {
    PersonalStorageItemResult {
        item_id: candidate.item_id.clone(),
        outcome: PersonalStorageOutcome::Unconfirmed,
        deleted_logical_bytes: None,
        deleted_file_count: None,
        retained_relative_path: path,
        reason: Some(reason.into()),
    }
}
fn results(items: Vec<PersonalStorageItemResult>) -> PersonalStorageResult {
    let size = items.iter().try_fold(0_u64, |sum, item| {
        sum.checked_add(item.deleted_logical_bytes?)
    });
    let count = items
        .iter()
        .try_fold(0_u32, |sum, item| sum.checked_add(item.deleted_file_count?));
    let mut issues = items
        .iter()
        .filter_map(|item| item.reason.clone())
        .collect::<Vec<_>>();
    issues.sort();
    issues.dedup();
    PersonalStorageResult {
        items,
        deleted_logical_bytes: size,
        deleted_file_count: count,
        physical_space_released: None,
        issues,
    }
}
fn missing(result: &std::io::Result<fs::Metadata>) -> bool {
    result
        .as_ref()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}
fn relative_staging(operation: &str, item: &str) -> String {
    format!("personal-storage-operations/{operation}/items/{item}")
}
fn staging_root(profile: &Path, operation: &str) -> Result<PathBuf, ServiceError> {
    if !operation.strip_prefix("storage_").is_some_and(identifier) {
        return Err(error("personal_storage_record_invalid"));
    }
    Ok(profile
        .join("personal-storage-operations")
        .join(operation)
        .join("items"))
}
fn restricted_directory(path: &Path) -> Result<(), ServiceError> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            restricted_directory(parent)?;
        }
        directory(parent)?;
    }
    security::ensure_restricted_directory(path)
        .map_err(|_| error("personal_storage_staging_failed"))?;
    directory(path)
}
fn same_node(path: &Path, node: &Node) -> Result<(), ServiceError> {
    if let Some(parent) = path.parent() {
        directory(parent)?;
    }
    let m = metadata(path)?;
    if m.stable_identity != node.identity
        || m.metadata.is_dir() != node.directory
        || (!node.directory
            && (m.metadata.len() != node.bytes
                || content_hash(path, &m)?.as_str()
                    != node
                        .digest
                        .as_deref()
                        .ok_or_else(|| error("personal_storage_record_invalid"))?))
    {
        return Err(error("personal_storage_content_changed"));
    }
    Ok(())
}
fn delete_frozen_tree(
    profile: &Path,
    staging: &Path,
    candidate: &Candidate,
) -> Result<(u64, u32), ServiceError> {
    if !safe_relative(&candidate.relative_path)
        || !candidate
            .item_id
            .strip_prefix("storage_item_")
            .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(error("personal_storage_record_invalid"));
    }
    let source = profile.join(&candidate.relative_path);
    let target = staging.join(&candidate.item_id);
    if snapshot(&source, true)? != candidate.nodes {
        return Err(error("personal_storage_content_changed"));
    }
    if !missing(&fs::symlink_metadata(&target)) {
        return Err(error("personal_storage_staging_occupied"));
    }
    directory(
        source
            .parent()
            .ok_or_else(|| error("personal_storage_record_invalid"))?,
    )?;
    directory(staging)?;
    fs::rename(&source, &target).map_err(|_| error("personal_storage_staging_failed"))?;
    filesystem::sync_directory(staging).map_err(|_| error("personal_storage_staging_failed"))?;
    filesystem::sync_directory(
        source
            .parent()
            .ok_or_else(|| error("personal_storage_record_invalid"))?,
    )
    .map_err(|_| error("personal_storage_staging_failed"))?;
    if snapshot(&target, true)? != candidate.nodes {
        return Err(error("personal_storage_content_changed"));
    }
    let mut ordered = candidate.nodes.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| {
        b.relative_path
            .matches('/')
            .count()
            .cmp(&a.relative_path.matches('/').count())
            .then_with(|| b.relative_path.len().cmp(&a.relative_path.len()))
    });
    let (mut size, mut count) = (0_u64, 0_u32);
    for node in ordered {
        let path = if node.relative_path.is_empty() {
            target.clone()
        } else {
            target.join(&node.relative_path)
        };
        same_node(&path, node)?;
        if node.directory {
            fs::remove_dir(&path).map_err(|_| error("personal_storage_delete_failed"))?;
        } else {
            fs::remove_file(&path).map_err(|_| error("personal_storage_delete_failed"))?;
            size = size
                .checked_add(node.bytes)
                .ok_or_else(|| error("personal_storage_scan_limit"))?;
            count = count
                .checked_add(1)
                .ok_or_else(|| error("personal_storage_scan_limit"))?;
        }
        filesystem::sync_directory(
            path.parent()
                .ok_or_else(|| error("personal_storage_record_invalid"))?,
        )
        .map_err(|_| error("personal_storage_delete_failed"))?;
    }
    Ok((size, count))
}
fn delete_candidate(
    profile: &Path,
    staging: &Path,
    candidate: &Candidate,
) -> PersonalStorageItemResult {
    match delete_frozen_tree(profile, staging, candidate) {
        Ok((size, count)) => PersonalStorageItemResult {
            item_id: candidate.item_id.clone(),
            outcome: PersonalStorageOutcome::Deleted,
            deleted_logical_bytes: Some(size),
            deleted_file_count: Some(count),
            retained_relative_path: None,
            reason: None,
        },
        Err(reason) => {
            let staged = staging.join(&candidate.item_id);
            if fs::symlink_metadata(&staged).is_ok() {
                let relative = staging
                    .strip_prefix(profile)
                    .ok()
                    .and_then(Path::to_str)
                    .map(|p| format!("{p}/{}", candidate.item_id));
                uncertain(candidate, relative, reason.reason)
            } else if snapshot(&profile.join(&candidate.relative_path), true)
                .is_ok_and(|nodes| nodes == candidate.nodes)
            {
                PersonalStorageItemResult {
                    item_id: candidate.item_id.clone(),
                    outcome: PersonalStorageOutcome::Retained,
                    deleted_logical_bytes: Some(0),
                    deleted_file_count: Some(0),
                    retained_relative_path: Some(candidate.relative_path.clone()),
                    reason: Some(reason.reason.into()),
                }
            } else {
                uncertain(candidate, None, reason.reason)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_persistence_sqlite::SqlitePersistence;
    use contexthub_protocol::{ModelExecutionOwner, PersonalDataQuiescence};
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
    };
    struct Fixture {
        root: PathBuf,
        profile: PathBuf,
        id: String,
        services: AppServerServices,
        owner: ModelExecutionOwner,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!(
                    "cilai-personal-storage-{}",
                    uuid::Uuid::new_v4().simple()
                ))
                .join("cn.cilai.personal.workbench/business");
            let id = uuid::Uuid::new_v4().simple().to_string();
            let profile = root.join("profiles").join(&id);
            security::ensure_restricted_directory(&profile).unwrap();
            fs::write(
                root.join("local-profile.json"),
                bytes(&json!({"schemaVersion":2,"profileId":id,"displayName":"存储测试"})).unwrap(),
            )
            .unwrap();
            let mut db = SqlitePersistence::open(&profile.join("contexthub.db"), "test").unwrap();
            db.begin_personal_profile_upgrade(&id, "test-instance")
                .unwrap();
            db.finish_personal_profile_upgrade(&id, "test-instance", "test-digest")
                .unwrap();
            Self {
                root,
                profile: profile.clone(),
                id,
                services: AppServerServices::with_data_dir(Some(db), Some(profile)),
                owner: ModelExecutionOwner {
                    instance_id: "test-host".into(),
                    pid: std::process::id(),
                    start_fingerprint:
                        contexthub_platform_adapter::instance::process_start_fingerprint(
                            std::process::id(),
                        )
                        .unwrap()
                        .unwrap(),
                },
            }
        }
        fn write(&self, path: &str, content: &[u8]) {
            let target = self.profile.join(path);
            security::ensure_restricted_directory(target.parent().unwrap()).unwrap();
            fs::write(target, content).unwrap();
        }
        fn cache(&self) {
            self.write("ai-runtime/skill-snapshots/old/SKILL.md", b"cached");
        }
        fn prepare(&mut self, path: &str) -> PersonalStorageOperation {
            let scan = self.services.storage_scan(1).unwrap();
            let item = scan
                .inventory
                .items
                .iter()
                .find(|i| i.managed_relative_path.as_deref() == Some(path))
                .unwrap();
            self.services
                .storage_prepare(
                    PersonalStoragePrepareParams {
                        item_ids: vec![item.item_id.clone()],
                        client_operation_id: uuid::Uuid::new_v4().to_string(),
                    },
                    1,
                )
                .unwrap()
        }
        fn proof(&mut self, op: &PersonalStorageOperation) -> PersonalDataQuiescence {
            let fence=self.services.persistence.as_mut().unwrap().personal_writer_rpc("personalMaintenance/acquire",json!({"operationId":op.operation_id,"previewDigest":op.preview_digest,"owner":self.owner}),&self.id,"test-instance",2).unwrap();
            PersonalDataQuiescence {
                protocol_version: 1,
                profile_id: self.id.clone(),
                owner: self.owner.clone(),
                token: fence["fenceId"].as_str().unwrap().into(),
                writer_registry_digest: hash(&Vec::<Value>::new()).unwrap(),
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
            }
        }
        fn execute(
            &mut self,
            op: &PersonalStorageOperation,
            proof: Option<PersonalDataQuiescence>,
        ) -> PersonalStorageOperation {
            self.services
                .storage_execute(
                    &self.profile,
                    &self.id,
                    &PersonalStorageExecuteParams {
                        operation_id: op.operation_id.clone(),
                        preview_digest: op.preview_digest.clone(),
                        client_operation_id: "execute".into(),
                        quiescence: proof,
                    },
                    "test-instance",
                    3,
                )
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.services.persistence.take();
            fs::remove_dir_all(self.root.parent().unwrap().parent().unwrap()).unwrap();
        }
    }
    #[test]
    fn inventory_protects_material_recovery_and_private_home_and_reports_unscanned_namespace() {
        let f = Fixture::new();
        f.cache();
        for p in [
            "attachments/source.txt",
            "personal-capability-data/app/db",
            "personal-capability-preview-data/app/input",
            "ai-history-backup/old.db",
            "ai-runtime/mcp-runtime/mcp1/lease1/env",
            "model-credential-operation.lock",
        ] {
            f.write(p, b"keep");
        }
        let inventory = f.services.storage_scan(1).unwrap().inventory;
        assert!(!inventory.size_complete);
        assert!(
            inventory
                .coverage
                .iter()
                .any(|c| c.scope == "application_namespace_browser_cache"
                    && c.state == PersonalStorageCoverageState::NotScanned)
        );
        for path in [
            "attachments",
            "personal-capability-data",
            "personal-capability-preview-data",
            "ai-history-backup",
            "ai-runtime/mcp-runtime",
            "model-credential-operation.lock",
            "contexthub.db",
        ] {
            assert!(
                !inventory
                    .items
                    .iter()
                    .find(|i| i.managed_relative_path.as_deref() == Some(path))
                    .unwrap()
                    .cleanup_allowed
            );
        }
        assert!(
            inventory
                .items
                .iter()
                .find(|i| i.managed_relative_path.as_deref() == Some("ai-runtime/skill-snapshots"))
                .unwrap()
                .cleanup_allowed
        );
    }
    #[test]
    fn missing_proof_records_incomplete_and_preserves_source() {
        let mut f = Fixture::new();
        f.cache();
        let op = f.prepare("ai-runtime/skill-snapshots");
        let result = f.execute(&op, None);
        assert_eq!(result.state, PersonalStorageOperationState::Incomplete);
        assert_eq!(result.result.unwrap().deleted_logical_bytes, Some(0));
        assert_eq!(
            fs::read(f.profile.join("ai-runtime/skill-snapshots/old/SKILL.md")).unwrap(),
            b"cached"
        );
    }
    #[test]
    fn checked_cleanup_deletes_only_selected_cache_and_repeated_request_reads_receipt() {
        let mut f = Fixture::new();
        f.cache();
        f.write("attachments/source.txt", b"keep");
        let op = f.prepare("ai-runtime/skill-snapshots");
        let proof = f.proof(&op);
        let result = f.execute(&op, Some(proof));
        assert_eq!(result.state, PersonalStorageOperationState::Completed);
        let receipt = result.result.as_ref().unwrap();
        assert_eq!(receipt.deleted_logical_bytes, Some(6));
        assert_eq!(receipt.deleted_file_count, Some(1));
        assert_eq!(receipt.physical_space_released, None);
        assert!(!f.profile.join("ai-runtime/skill-snapshots").exists());
        assert_eq!(
            fs::read(f.profile.join("attachments/source.txt")).unwrap(),
            b"keep"
        );
        assert_eq!(f.execute(&op, None), result);
        let status = f
            .services
            .persistence
            .as_mut()
            .unwrap()
            .personal_writer_rpc(
                "personalMaintenance/operationStatus",
                json!({"operationId":op.operation_id}),
                &f.id,
                "test-instance",
                4,
            )
            .unwrap();
        assert_eq!(status["kind"], "storage_cleanup");
        assert_eq!(status["state"], "completed");
    }
    #[test]
    fn changed_content_after_preview_is_retained() {
        let mut f = Fixture::new();
        f.cache();
        let op = f.prepare("ai-runtime/skill-snapshots");
        f.write("ai-runtime/skill-snapshots/old/SKILL.md", b"changed");
        let proof = f.proof(&op);
        let result = f.execute(&op, Some(proof));
        assert_eq!(result.state, PersonalStorageOperationState::Incomplete);
        assert_eq!(result.blockers, vec!["personal_storage_content_changed"]);
        assert_eq!(
            fs::read(f.profile.join("ai-runtime/skill-snapshots/old/SKILL.md")).unwrap(),
            b"changed"
        );
    }
    #[test]
    fn protected_selection_never_deletes_even_with_proof() {
        let mut f = Fixture::new();
        f.write("attachments/user.txt", b"keep");
        let op = f.prepare("attachments");
        assert!(!op.blockers.is_empty());
        let proof = f.proof(&op);
        assert_eq!(
            f.execute(&op, Some(proof)).state,
            PersonalStorageOperationState::Incomplete
        );
        assert!(f.profile.join("attachments/user.txt").exists());
    }
    #[test]
    fn incomplete_does_not_retry_with_new_proof() {
        let mut f = Fixture::new();
        f.cache();
        let op = f.prepare("ai-runtime/skill-snapshots");
        let result = f.execute(&op, None);
        let proof = f.proof(&op);
        let error = f
            .services
            .storage_execute(
                &f.profile,
                &f.id,
                &PersonalStorageExecuteParams {
                    operation_id: op.operation_id,
                    preview_digest: op.preview_digest,
                    client_operation_id: "retry".into(),
                    quiescence: Some(proof),
                },
                "test-instance",
                4,
            )
            .unwrap_err();
        assert_eq!(error.reason, "personal_storage_operation_unconfirmed");
        assert_eq!(
            f.services
                .storage_read(&f.id, &result.operation_id)
                .unwrap()
                .operation,
            result
        );
    }
    #[test]
    fn referenced_skill_package_is_protected_while_old_unreferenced_version_is_eligible() {
        let f = Fixture::new();
        let live = uuid::Uuid::new_v4().simple().to_string();
        let old = uuid::Uuid::new_v4().simple().to_string();
        f.write(
            &format!("skills/user-packages/skill_test/{live}/SKILL.md"),
            b"live",
        );
        f.write(
            &format!("skills/user-packages/skill_test/{old}/SKILL.md"),
            b"old",
        );
        let db = rusqlite::Connection::open(f.profile.join("contexthub.db")).unwrap();
        db.execute("INSERT INTO user_skills(skill_id,name,normalized_name,description,managed_relative_path,content_digest,created_at,updated_at) VALUES('skill_test','test','test','test',?1,'sha256:test',1,1)",[format!("user-packages/skill_test/{live}/SKILL.md")]).unwrap();
        let scan = f.services.storage_scan(1).unwrap();
        let live_item = scan
            .inventory
            .items
            .iter()
            .find(|i| i.display_path.contains(&live))
            .unwrap();
        assert!(!live_item.cleanup_allowed);
        assert_eq!(live_item.references[0].kind, "user_skills");
        assert!(
            scan.inventory
                .items
                .iter()
                .find(|i| i.display_path.contains(&old))
                .unwrap()
                .cleanup_allowed
        );
    }
    #[test]
    fn active_registered_writer_blocks_cleanup_even_when_proof_claims_no_writers() {
        let mut f = Fixture::new();
        f.cache();
        let op = f.prepare("ai-runtime/skill-snapshots");
        f.services.persistence.as_mut().unwrap().personal_writer_rpc("profileWriter/prepare",json!({"clientOperationId":"busy-writer","kind":"user_mcp","resourceId":"busy","resourceRevision":"one","owner":f.owner,"launchProtocol":1,"processMode":"remote_request"}),&f.id,"test-instance",2).unwrap();
        let proof = f.proof(&op);
        let result = f.execute(&op, Some(proof));
        assert_eq!(result.state, PersonalStorageOperationState::Incomplete);
        assert_eq!(result.blockers, vec!["personal_data_writer_active"]);
        assert!(
            f.profile
                .join("ai-runtime/skill-snapshots/old/SKILL.md")
                .exists()
        );
    }
    #[test]
    fn candidate_change_before_move_preserves_every_new_file() {
        let f = Fixture::new();
        f.cache();
        let scan = f.services.storage_scan(1).unwrap();
        let candidate = scan.candidates.first().unwrap();
        f.write("ai-runtime/skill-snapshots/new.txt", b"new");
        let staging = staging_root(&f.profile, "storage_11111111111111111111111111111111").unwrap();
        restricted_directory(&staging).unwrap();
        let result = delete_candidate(&f.profile, &staging, candidate);
        assert_ne!(result.outcome, PersonalStorageOutcome::Deleted);
        assert!(
            f.profile
                .join("ai-runtime/skill-snapshots/new.txt")
                .exists()
        );
        assert!(
            f.profile
                .join("ai-runtime/skill-snapshots/old/SKILL.md")
                .exists()
        );
    }
    #[test]
    fn absent_paths_without_a_persisted_delete_receipt_remain_unconfirmed() {
        let f = Fixture::new();
        f.cache();
        let scan = f.services.storage_scan(1).unwrap();
        let candidate = scan.candidates.first().unwrap();
        let operation = "storage_11111111111111111111111111111111";
        let staging = staging_root(&f.profile, operation).unwrap();
        fs::rename(
            f.profile.join(&candidate.relative_path),
            f.profile.join("kept_elsewhere"),
        )
        .unwrap();
        let result = reconcile_item(&f.profile, &staging, candidate, None, operation);
        assert_eq!(result.outcome, PersonalStorageOutcome::Unconfirmed);
        assert_eq!(result.deleted_logical_bytes, None);
        assert!(f.profile.join("kept_elsewhere/old/SKILL.md").exists());
    }
    #[test]
    fn invalid_operation_path_is_rejected_before_creating_staging() {
        let f = Fixture::new();
        assert!(staging_root(&f.profile, "../outside").is_err());
        assert!(!f.profile.join("personal-storage-operations").exists());
    }
    #[test]
    fn live_owner_cannot_resolve_an_applying_operation() {
        let mut f = Fixture::new();
        f.cache();
        let mut op = f.prepare("ai-runtime/skill-snapshots");
        let _proof = f.proof(&op);
        op.state = PersonalStorageOperationState::Applying;
        f.services
            .storage_transition(&op, &PersonalStorageOperationState::Prepared)
            .unwrap();
        let result = f
            .services
            .storage_resolve_interrupted(
                &f.profile,
                &f.id,
                &PersonalStorageConfirmParams {
                    operation_id: op.operation_id,
                    preview_digest: op.preview_digest,
                },
                4,
            )
            .unwrap_err();
        assert_eq!(result.reason, "personal_maintenance_owner_alive");
        assert!(
            f.profile
                .join("ai-runtime/skill-snapshots/old/SKILL.md")
                .exists()
        );
    }
    #[test]
    fn interrupted_staged_cleanup_reports_unknown_size_and_keeps_remaining_content() {
        let mut f = Fixture::new();
        f.cache();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "personal_storage::tests::storage_owner_child",
                "--ignored",
                "--nocapture",
            ])
            .env("CILAI_STORAGE_OWNER_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert!(output.read_line(&mut line).unwrap() > 0);
            if line.contains("storage-owner-ready") {
                break;
            }
        }
        f.owner.pid = child.id();
        f.owner.start_fingerprint =
            contexthub_platform_adapter::instance::process_start_fingerprint(child.id())
                .unwrap()
                .unwrap();
        let mut op = f.prepare("ai-runtime/skill-snapshots");
        let _proof = f.proof(&op);
        op.state = PersonalStorageOperationState::Applying;
        f.services
            .storage_transition(&op, &PersonalStorageOperationState::Prepared)
            .unwrap();
        let staging = staging_root(&f.profile, &op.operation_id).unwrap();
        restricted_directory(&staging).unwrap();
        let record = f.services.storage_read(&f.id, &op.operation_id).unwrap();
        let candidates: Vec<Candidate> = serde_json::from_value(record.candidates).unwrap();
        fs::rename(
            f.profile.join(&candidates[0].relative_path),
            staging.join(&candidates[0].item_id),
        )
        .unwrap();
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
        let result = f
            .services
            .storage_resolve_interrupted(
                &f.profile,
                &f.id,
                &PersonalStorageConfirmParams {
                    operation_id: op.operation_id,
                    preview_digest: op.preview_digest,
                },
                4,
            )
            .unwrap();
        assert_eq!(result.state, PersonalStorageOperationState::Incomplete);
        assert_eq!(result.result.as_ref().unwrap().deleted_logical_bytes, None);
        let retained = &result.result.as_ref().unwrap().items[0].retained_relative_path;
        assert!(
            f.profile
                .join(retained.as_ref().unwrap())
                .join("old/SKILL.md")
                .exists()
        );
    }
    #[test]
    #[ignore = "由实际父测试启动并等待退出"]
    fn storage_owner_child() {
        if std::env::var("CILAI_STORAGE_OWNER_CHILD").as_deref() != Ok("1") {
            return;
        }
        println!("storage-owner-ready");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
    }
}
