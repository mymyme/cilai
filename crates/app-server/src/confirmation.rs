use crate::workspace::{ProjectionAttempt, attempt_projection};
use crate::{preview::classify_preview_format, text_diff::DEFAULT_MAX_DIFF_SOURCE_BYTES};
use contexthub_application::{
    ConfirmationRepository, CreateFileConfirmationCommand, FileConfirmationRecord, UnitOfWork,
    create_file_confirmation,
};
use contexthub_domain::{AccountId, EventId, WorkspaceId, WorkspaceRelativePath};
use contexthub_persistence_sqlite::{PersistenceError, SqlitePersistence};
use contexthub_protocol::{
    ConfirmFileParams, ConfirmFileResult, ConfirmationHistoryItem, ConfirmationHistoryParams,
    ConfirmationHistoryResult, PreviewFormat,
};
use contexthub_workspace_fs::{
    ExpectedTextResourceRevision, MAX_TEXT_CHUNK_BYTES, WorkspaceChecksumCancellationToken,
    WorkspaceChecksumErrorKind, WorkspaceChecksumOutcome, checksum_workspace_file,
    open_text_resource,
};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::UNIX_EPOCH;

const LOCAL_CONFIRMATION_IDENTITY: &str = "local-profile:default";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmFileError {
    InvalidRequest,
    FileNotFound,
    FileNotReady,
    ChecksumMismatch,
    ContentChanged,
    BaselineChanged,
    StateChanged,
    NotAllowed,
    IdentityUnavailable,
    IdempotencyConflict,
    ChecksumFailed,
    SnapshotFailed,
    PersistenceFailed,
    ProjectionFailed,
}

impl ConfirmFileError {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_file_confirmation_request",
            Self::FileNotFound => "confirmation_file_not_found",
            Self::FileNotReady => "confirmation_file_not_ready",
            Self::ChecksumMismatch => "confirmation_checksum_mismatch",
            Self::ContentChanged => "content_changed_during_confirmation",
            Self::BaselineChanged => "confirmation_baseline_changed",
            Self::StateChanged => "confirmation_state_changed",
            Self::NotAllowed => "confirmation_not_allowed",
            Self::IdentityUnavailable => "confirmation_identity_unavailable",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::ChecksumFailed => "confirmation_checksum_failed",
            Self::SnapshotFailed => "confirmation_snapshot_failed",
            Self::PersistenceFailed => "confirmation_persistence_failed",
            Self::ProjectionFailed => "confirmation_projection_failed",
        }
    }
}

/// 对指定本地文件执行强 checksum 校验，并以单事务创建确认、状态和耐久事件。
///
/// # Errors
///
/// 参数、文件、摘要、基线、能力、幂等性、持久化或清单投影失败时返回稳定错误。
pub fn confirm_file(
    persistence: &mut SqlitePersistence,
    params: &ConfirmFileParams,
    source_client: &str,
    now: i64,
) -> Result<ConfirmFileResult, ConfirmFileError> {
    validate_params(params, source_client, now)?;
    let workspace_id = WorkspaceId::new(params.workspace_id.clone());
    let request_fingerprint = request_fingerprint(params, source_client);
    if let Some(record) = persistence
        .transact(|repositories| {
            repositories
                .file_confirmation_for_operation(&params.client_operation_id, &request_fingerprint)
        })
        .map_err(|error| map_persistence_error(&error))?
    {
        return finish_confirmation(persistence, &workspace_id, now, record);
    }
    let candidate = persistence
        .transact(|repositories| {
            repositories.confirmation_candidate(&workspace_id, &params.local_file_id)
        })
        .map_err(|_| ConfirmFileError::PersistenceFailed)?
        .ok_or(ConfirmFileError::FileNotFound)?;
    if candidate.workspace_is_cloud {
        return Err(ConfirmFileError::NotAllowed);
    }
    if candidate.materialization_state != "materialized"
        || candidate.file_type != "file"
        || candidate.scan_state != "current"
    {
        return Err(ConfirmFileError::FileNotReady);
    }
    let path = WorkspaceRelativePath::parse(&candidate.relative_path)
        .map_err(|_| ConfirmFileError::InvalidRequest)?;
    let outcome = checksum_workspace_file(
        Path::new(&candidate.workspace_root),
        &path,
        &WorkspaceChecksumCancellationToken::new(),
        |_| {},
    )
    .map_err(|error| map_checksum_error(&error))?;
    let WorkspaceChecksumOutcome::Completed(verified) = outcome else {
        return Err(ConfirmFileError::ChecksumFailed);
    };
    if verified.checksum.algorithm.as_str() != params.expected_checksum_algorithm
        || verified.checksum.value != params.expected_checksum_value
    {
        return Err(ConfirmFileError::ChecksumMismatch);
    }
    let verified_size_bytes = i64::try_from(verified.observation.size_bytes)
        .map_err(|_| ConfirmFileError::InvalidRequest)?;
    let verified_modified_at_fs = verified
        .observation
        .modified_at
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .ok_or(ConfirmFileError::InvalidRequest)?;
    let confirmed_text_content = read_confirmed_text_snapshot(
        &candidate.workspace_root,
        &candidate.relative_path,
        verified.observation.size_bytes,
        &params.expected_checksum_algorithm,
        &params.expected_checksum_value,
    )?;
    let confirmation_id = format!("confirmation-{}", uuid::Uuid::new_v4());
    let event_id = EventId::new(format!("event-{}", uuid::Uuid::new_v4()));
    let correlation_id = format!("correlation-{}", uuid::Uuid::new_v4());
    let record = persistence
        .transact(|repositories| {
            create_file_confirmation(
                repositories,
                &CreateFileConfirmationCommand {
                    confirmation_id,
                    workspace_id: workspace_id.clone(),
                    local_file_id: params.local_file_id.clone(),
                    expected_local_file_revision: candidate.local_file_revision,
                    expected_file_state_revision: candidate.file_state_revision,
                    expected_checksum_algorithm: params.expected_checksum_algorithm.clone(),
                    expected_checksum_value: params.expected_checksum_value.clone(),
                    expected_baseline_version_id: params.expected_baseline_version_id.clone(),
                    verified_size_bytes,
                    verified_modified_at_fs,
                    confirmed_text_content,
                    confirmed_by_identity_id: AccountId::new(LOCAL_CONFIRMATION_IDENTITY),
                    source_client: source_client.to_owned(),
                    client_operation_id: params.client_operation_id.clone(),
                    request_fingerprint,
                    event_id,
                    correlation_id,
                    confirmed_at: now,
                },
            )
        })
        .map_err(|error| map_persistence_error(&error))?;
    finish_confirmation(persistence, &workspace_id, now, record)
}

fn read_confirmed_text_snapshot(
    workspace_root: &str,
    relative_path: &str,
    size_bytes: u64,
    checksum_algorithm: &str,
    checksum_value: &str,
) -> Result<Option<String>, ConfirmFileError> {
    if !matches!(
        classify_preview_format(relative_path),
        PreviewFormat::Markdown | PreviewFormat::Html | PreviewFormat::PlainText
    ) {
        return Ok(None);
    }
    let path = WorkspaceRelativePath::parse(relative_path)
        .map_err(|_| ConfirmFileError::InvalidRequest)?;
    let expected = ExpectedTextResourceRevision {
        size_bytes: Some(size_bytes),
        checksum_algorithm: Some(checksum_algorithm.to_owned()),
        checksum_value: Some(checksum_value.to_owned()),
    };
    let mut resource = open_text_resource(
        Path::new(workspace_root),
        &path,
        &expected,
        DEFAULT_MAX_DIFF_SOURCE_BYTES,
    )
    .map_err(|_| ConfirmFileError::SnapshotFailed)?;
    let mut content = String::with_capacity(
        usize::try_from(resource.metadata().content_bytes)
            .map_err(|_| ConfirmFileError::SnapshotFailed)?,
    );
    let mut offset = resource.metadata().first_offset;
    loop {
        let chunk = resource
            .read_chunk(offset, MAX_TEXT_CHUNK_BYTES)
            .map_err(|_| ConfirmFileError::SnapshotFailed)?;
        content.push_str(&chunk.text);
        let Some(next) = chunk.next_offset else { break };
        offset = next;
    }
    resource
        .verify_unchanged()
        .map_err(|_| ConfirmFileError::SnapshotFailed)?;
    Ok(Some(content))
}

/// 读取指定文件的完整确认审计历史。
///
/// # Errors
///
/// 参数、文件身份或持久化读取失败时返回稳定错误。
pub fn confirmation_history(
    persistence: &mut SqlitePersistence,
    params: &ConfirmationHistoryParams,
) -> Result<ConfirmationHistoryResult, ConfirmFileError> {
    if params.workspace_id.is_empty() || params.local_file_id.is_empty() {
        return Err(ConfirmFileError::InvalidRequest);
    }
    let workspace_id = WorkspaceId::new(params.workspace_id.clone());
    let items = persistence
        .transact(|repositories| {
            repositories.file_confirmation_history(&workspace_id, &params.local_file_id)
        })
        .map_err(|error| map_persistence_error(&error))?
        .into_iter()
        .map(|record| ConfirmationHistoryItem {
            confirmation_id: record.confirmation_id,
            checksum_algorithm: record.checksum_algorithm,
            checksum_value: record.checksum_value,
            baseline_version_id: record.baseline_version_id,
            confirmed_by: record.confirmed_by,
            confirmed_at: record.confirmed_at,
            invalidated_at: record.invalidated_at,
            invalidation_reason: record.invalidation_reason,
        })
        .collect();
    Ok(ConfirmationHistoryResult {
        workspace_id: params.workspace_id.clone(),
        local_file_id: params.local_file_id.clone(),
        items,
    })
}

fn finish_confirmation(
    persistence: &mut SqlitePersistence,
    workspace_id: &WorkspaceId,
    now: i64,
    record: FileConfirmationRecord,
) -> Result<ConfirmFileResult, ConfirmFileError> {
    if matches!(
        attempt_projection(persistence, workspace_id, now),
        ProjectionAttempt::Failed { .. }
    ) {
        return Err(ConfirmFileError::ProjectionFailed);
    }
    Ok(protocol_result(record))
}

fn validate_params(
    params: &ConfirmFileParams,
    source_client: &str,
    now: i64,
) -> Result<(), ConfirmFileError> {
    if params.workspace_id.is_empty()
        || params.local_file_id.is_empty()
        || params.expected_checksum_algorithm != "sha256"
        || params.expected_checksum_value.len() != 64
        || !params
            .expected_checksum_value
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        || params.client_operation_id.is_empty()
        || params.client_operation_id.len() > 128
        || !matches!(source_client, "desktop" | "cli")
        || now < 0
    {
        return Err(ConfirmFileError::InvalidRequest);
    }
    Ok(())
}

fn request_fingerprint(params: &ConfirmFileParams, source_client: &str) -> String {
    let mut hasher = Sha256::new();
    for value in [
        params.workspace_id.as_str(),
        params.local_file_id.as_str(),
        params.expected_checksum_algorithm.as_str(),
        params.expected_checksum_value.as_str(),
        params.expected_baseline_version_id.as_deref().unwrap_or(""),
        source_client,
    ] {
        hasher.update(value.len().to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn map_checksum_error(error: &contexthub_workspace_fs::WorkspaceChecksumError) -> ConfirmFileError {
    match error.kind() {
        WorkspaceChecksumErrorKind::ChangedDuringRead => ConfirmFileError::ContentChanged,
        WorkspaceChecksumErrorKind::TargetUnavailable
        | WorkspaceChecksumErrorKind::NotRegularFile
        | WorkspaceChecksumErrorKind::SourceNotMaterialized => ConfirmFileError::FileNotReady,
        _ => ConfirmFileError::ChecksumFailed,
    }
}

fn map_persistence_error(error: &PersistenceError) -> ConfirmFileError {
    match error.reason() {
        Some("confirmation_file_not_found") => ConfirmFileError::FileNotFound,
        Some("confirmation_file_not_ready") => ConfirmFileError::FileNotReady,
        Some("content_changed_during_confirmation") => ConfirmFileError::ContentChanged,
        Some("confirmation_baseline_changed") => ConfirmFileError::BaselineChanged,
        Some("confirmation_state_changed") => ConfirmFileError::StateChanged,
        Some("confirmation_not_allowed") => ConfirmFileError::NotAllowed,
        Some("confirmation_identity_unavailable") => ConfirmFileError::IdentityUnavailable,
        Some("idempotency_conflict") => ConfirmFileError::IdempotencyConflict,
        _ => ConfirmFileError::PersistenceFailed,
    }
}

fn protocol_result(record: FileConfirmationRecord) -> ConfirmFileResult {
    ConfirmFileResult {
        confirmation_id: record.confirmation_id,
        workspace_id: record.workspace_id.as_str().to_owned(),
        local_file_id: record.local_file_id,
        checksum_algorithm: record.checksum_algorithm,
        checksum_value: record.checksum_value,
        baseline_version_id: record.baseline_version_id,
        confirmed_by: record.confirmed_by,
        confirmed_at: record.confirmed_at,
        replayed: record.replayed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{CaptureLocalSnapshotCommand, capture_full_local_snapshot};
    use contexthub_application::{
        PreviewRepository, WorkspaceExportRecord, WorkspaceRecord, WorkspaceRepository,
    };
    use contexthub_domain::{LocalWorkspaceStatus, WorkspaceKind};
    use contexthub_workspace_fs::WorkspaceCancellationToken;
    use std::fs;

    #[test]
    fn confirms_local_file_idempotently_and_rejects_changed_requests_and_content() {
        let root = std::env::temp_dir().join(format!("ch-confirm-{}", uuid::Uuid::new_v4()));
        let data = root.join("data");
        let workspace = root.join("workspace");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("note.md"), "confirmed content").unwrap();
        let mut persistence =
            SqlitePersistence::open(&data.join("contexthub.db"), "0.1.0").unwrap();
        insert_local_workspace(&mut persistence, &workspace);
        let snapshot = capture_full_local_snapshot(
            &mut persistence,
            &CaptureLocalSnapshotCommand {
                workspace_id: WorkspaceId::new("w1"),
                snapshot_id: "snapshot-1".to_owned(),
                event_id: EventId::new("snapshot-event"),
                correlation_id: "snapshot-correlation".to_owned(),
                observed_at: 2,
                completed_at: 3,
            },
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        let file_id = snapshot.business.created_local_file_ids[0].clone();
        let checksum = format!("{:x}", Sha256::digest(b"confirmed content"));
        let params = ConfirmFileParams {
            workspace_id: "w1".to_owned(),
            local_file_id: file_id,
            expected_checksum_algorithm: "sha256".to_owned(),
            expected_checksum_value: checksum.clone(),
            expected_baseline_version_id: None,
            client_operation_id: "operation-1".to_owned(),
        };
        let created = confirm_file(&mut persistence, &params, "desktop", 4).unwrap();
        assert!(!created.replayed);
        assert_eq!(created.confirmed_by, "本机操作人");
        let preview = persistence
            .transact(|repository| {
                repository.preview_file(&WorkspaceId::new("w1"), &params.local_file_id)
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            preview.baseline_text_content.as_deref(),
            Some("confirmed content")
        );
        assert_eq!(
            preview.baseline_content_revision.as_deref(),
            Some(checksum.as_str())
        );
        assert_eq!(preview.baseline_confirmed_at, Some(4));
        let replayed = confirm_file(&mut persistence, &params, "desktop", 5).unwrap();
        assert!(replayed.replayed);
        assert_eq!(replayed.confirmation_id, created.confirmation_id);

        let mut conflict = params.clone();
        conflict.expected_baseline_version_id = Some("other-version".to_owned());
        assert_eq!(
            confirm_file(&mut persistence, &conflict, "desktop", 6).unwrap_err(),
            ConfirmFileError::IdempotencyConflict
        );
        fs::write(workspace.join("note.md"), "changed content").unwrap();
        let mut changed = params;
        changed.client_operation_id = "operation-2".to_owned();
        assert_eq!(
            confirm_file(&mut persistence, &changed, "desktop", 7).unwrap_err(),
            ConfirmFileError::ChecksumMismatch
        );
        capture_full_local_snapshot(
            &mut persistence,
            &CaptureLocalSnapshotCommand {
                workspace_id: WorkspaceId::new("w1"),
                snapshot_id: "snapshot-2".to_owned(),
                event_id: EventId::new("snapshot-event-2"),
                correlation_id: "snapshot-correlation-2".to_owned(),
                observed_at: 8,
                completed_at: 9,
            },
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        let history = confirmation_history(
            &mut persistence,
            &ConfirmationHistoryParams {
                workspace_id: "w1".to_owned(),
                local_file_id: changed.local_file_id,
            },
        )
        .unwrap();
        assert_eq!(history.items.len(), 1);
        assert_eq!(
            history.items[0].invalidation_reason.as_deref(),
            Some("local_content_changed")
        );
        assert_eq!(history.items[0].invalidated_at, Some(9));
        drop(persistence);
        fs::remove_dir_all(root).unwrap();
    }

    fn insert_local_workspace(persistence: &mut SqlitePersistence, workspace: &std::path::Path) {
        persistence
            .transact(|repositories| {
                repositories.insert_workspace(&WorkspaceRecord {
                    workspace_id: WorkspaceId::new("w1"),
                    workspace_kind: WorkspaceKind::Local,
                    display_name: "本地目录".to_owned(),
                    local_root: workspace.to_string_lossy().into_owned(),
                    local_root_identity: None,
                    local_status: LocalWorkspaceStatus::Ready,
                    state_revision: 0,
                    last_local_scan_at: None,
                    created_at: 1,
                    updated_at: 1,
                })?;
                repositories.initialize_workspace_export(&WorkspaceExportRecord {
                    workspace_id: WorkspaceId::new("w1"),
                    desired_revision: 0,
                    exported_revision: 0,
                    updated_at: 1,
                })
            })
            .unwrap();
    }
}
