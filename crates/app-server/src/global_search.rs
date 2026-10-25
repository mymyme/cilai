use contexthub_application::{GlobalSearchQuery, GlobalSearchRecord, WorkspaceRecord};
use contexthub_domain::{WorkspaceKind, WorkspaceRelativePath};
use contexthub_persistence_sqlite::{
    GlobalSearchReader, PersistenceError, PersistenceErrorKind, SqlitePersistence,
};
use contexthub_protocol::{GlobalSearchItem, GlobalSearchParams, GlobalSearchResult};
use contexthub_workspace_fs::{
    ExpectedTextResourceRevision, LocalWorkspaceRootErrorKind, TextResourceErrorKind,
    WorkspaceBrowseErrorKind, WorkspaceEntryKind, inspect_local_workspace_root,
    inspect_workspace_entry, open_text_resource,
};
use std::collections::HashMap;
use std::error::Error;
use std::io::ErrorKind;
use std::path::Path;

const MAX_SEARCH_TEXT_BYTES: u64 = 2 * 1024 * 1024;

/// 查询当前本地用户可访问的项目、阶段、任务、交付物、决策和文件来源事实。
///
/// # Errors
/// 查询参数或本地事实读取失败时返回稳定错误。
pub fn query(
    persistence: &mut SqlitePersistence,
    params: &GlobalSearchParams,
) -> Result<GlobalSearchResult, PersistenceError> {
    Ok(query_page(persistence, params, None)?.0)
}

pub(super) type SearchPosition = (i64, String, String);

pub(super) fn query_page(
    persistence: &mut SqlitePersistence,
    params: &GlobalSearchParams,
    after: Option<SearchPosition>,
) -> Result<(GlobalSearchResult, Option<SearchPosition>), PersistenceError> {
    query_reader_page(&mut persistence.global_search_reader()?, params, after)
}

pub(super) fn query_reader_page(
    reader: &mut GlobalSearchReader,
    params: &GlobalSearchParams,
    after: Option<SearchPosition>,
) -> Result<(GlobalSearchResult, Option<SearchPosition>), PersistenceError> {
    let value = params.query.trim();
    if !(2..=200).contains(&value.chars().count())
        || value.contains('\0')
        || !(1..=100).contains(&params.limit)
    {
        return Err(PersistenceError::with_reason(
            contexthub_persistence_sqlite::PersistenceErrorKind::Configuration,
            "global_search_params_invalid",
            "global_search_params_invalid",
        ));
    }
    let workspaces = reader.workspaces()?;
    let workspaces = workspaces
        .iter()
        .map(|workspace| (workspace.workspace_id.as_str(), workspace))
        .collect::<HashMap<_, _>>();
    let limit = params.limit as usize;
    let batch_limit = (limit + 1).max(32);
    let mut position = after;
    let mut records = Vec::with_capacity(limit + 1);
    loop {
        let batch = reader.search(&GlobalSearchQuery {
            query: value.to_owned(),
            limit: batch_limit,
            after: position.clone(),
        })?;
        let exhausted = batch.len() < batch_limit;
        position = batch.last().map(record_position);
        for record in batch {
            if source_is_current(&record, &workspaces)? {
                records.push(record);
                if records.len() > limit {
                    break;
                }
            }
        }
        if records.len() > limit || exhausted {
            break;
        }
    }
    let has_more = records.len() > limit;
    records.truncate(limit);
    let next = if has_more {
        records.last().map(record_position)
    } else {
        None
    };
    let items = records
        .into_iter()
        .map(|record| GlobalSearchItem {
            object_type: record.object_type,
            object_id: record.object_id,
            title: record.title,
            subtitle: record.subtitle,
            project_id: record.project_id,
            workspace_id: record.workspace_id,
            phase_id: record.phase_id,
            deliverable_id: record.deliverable_id,
            local_file_id: record.local_file_id,
            relative_path: record.relative_path,
            updated_at: record.updated_at,
        })
        .collect::<Vec<_>>();
    Ok((
        GlobalSearchResult {
            total: items.len() as u64,
            items,
        },
        next,
    ))
}

fn record_position(record: &GlobalSearchRecord) -> SearchPosition {
    (
        record.updated_at,
        record.object_type.clone(),
        record.object_id.clone(),
    )
}

fn source_is_current(
    record: &GlobalSearchRecord,
    workspaces: &HashMap<&str, &WorkspaceRecord>,
) -> Result<bool, PersistenceError> {
    if record.object_type != "file" {
        return Ok(true);
    }
    let Some(workspace) = workspaces.get(record.workspace_id.as_str()) else {
        return Ok(false);
    };
    if workspace.workspace_kind != WorkspaceKind::Local {
        return Ok(false);
    }
    if !workspace_root_is_current(workspace)? {
        return Ok(false);
    }
    let Some(path) = record
        .relative_path
        .as_deref()
        .and_then(|path| WorkspaceRelativePath::parse(path).ok())
    else {
        return Ok(false);
    };
    let root = Path::new(&workspace.local_root);
    let Some(entry) = inspect_current_entry(root, &path)? else {
        return Ok(false);
    };
    if record
        .expected_file_identity
        .as_ref()
        .is_some_and(|expected| entry.file_identity.as_ref() != Some(expected))
    {
        return Ok(false);
    }
    if !record.body_match {
        return workspace_root_is_current(workspace);
    }
    if entry.kind != WorkspaceEntryKind::File
        || record.expected_file_identity.is_none()
        || record.expected_size_bytes.is_none()
        || record.expected_checksum.is_none()
    {
        return Ok(false);
    }
    let opened = match open_text_resource(
        root,
        &path,
        &ExpectedTextResourceRevision {
            size_bytes: record.expected_size_bytes,
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: record.expected_checksum.clone(),
        },
        MAX_SEARCH_TEXT_BYTES,
    ) {
        Ok(opened) => opened,
        Err(error) => return classify_text_failure(&error),
    };
    let Some(current) = inspect_current_entry(root, &path)? else {
        return Ok(false);
    };
    if current.file_identity != entry.file_identity {
        return Ok(false);
    }
    match opened.verify_unchanged() {
        Ok(()) => workspace_root_is_current(workspace),
        Err(error) => classify_text_failure(&error),
    }
}

fn workspace_root_is_current(workspace: &WorkspaceRecord) -> Result<bool, PersistenceError> {
    match inspect_local_workspace_root(Path::new(&workspace.local_root), &[]) {
        Ok(current) => Ok(current.canonical_root == workspace.local_root
            && workspace.local_root_identity.is_some()
            && current.local_root_identity == workspace.local_root_identity),
        Err(error)
            if is_missing_io(&error)
                || error.kind() == LocalWorkspaceRootErrorKind::RootNotDirectory =>
        {
            Ok(false)
        }
        Err(_) => Err(incomplete_search()),
    }
}

fn inspect_current_entry(
    root: &Path,
    path: &WorkspaceRelativePath,
) -> Result<Option<contexthub_workspace_fs::WorkspaceScanEntry>, PersistenceError> {
    match inspect_workspace_entry(root, path) {
        Ok(entry) if entry.kind != WorkspaceEntryKind::Symlink => Ok(Some(entry)),
        Ok(_) => Ok(None),
        Err(error) => match error.kind() {
            WorkspaceBrowseErrorKind::OutsideWorkspace
            | WorkspaceBrowseErrorKind::NotDirectory
            | WorkspaceBrowseErrorKind::Ignored
            | WorkspaceBrowseErrorKind::UnsupportedFileType => Ok(None),
            WorkspaceBrowseErrorKind::RootUnavailable
            | WorkspaceBrowseErrorKind::TargetUnavailable => {
                if is_missing_io(&error) {
                    Ok(None)
                } else {
                    Err(incomplete_search())
                }
            }
        },
    }
}

fn classify_text_failure(
    error: &contexthub_workspace_fs::TextResourceError,
) -> Result<bool, PersistenceError> {
    if let Some(kind) = source_io_kind(error) {
        return if matches!(kind, ErrorKind::NotFound | ErrorKind::NotADirectory) {
            Ok(false)
        } else {
            Err(incomplete_search())
        };
    }
    match error.kind() {
        TextResourceErrorKind::OutsideWorkspace
        | TextResourceErrorKind::ReservedPath
        | TextResourceErrorKind::NotRegularFile
        | TextResourceErrorKind::FileTooLarge
        | TextResourceErrorKind::BinaryContent
        | TextResourceErrorKind::UnsupportedEncoding
        | TextResourceErrorKind::SourceChanged => Ok(false),
        TextResourceErrorKind::RootUnavailable
        | TextResourceErrorKind::TargetUnavailable
        | TextResourceErrorKind::InvalidOffset
        | TextResourceErrorKind::InvalidChunkLimit
        | TextResourceErrorKind::SourceNotMaterialized
        | TextResourceErrorKind::ReadFailed => Err(incomplete_search()),
    }
}

fn source_io_kind(mut error: &(dyn Error + 'static)) -> Option<ErrorKind> {
    loop {
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            return Some(error.kind());
        }
        error = error.source()?;
    }
}

fn is_missing_io(error: &(dyn Error + 'static)) -> bool {
    matches!(
        source_io_kind(error),
        Some(ErrorKind::NotFound | ErrorKind::NotADirectory)
    )
}

fn incomplete_search() -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Configuration,
        "global_search_incomplete",
        "local search could not verify an indexed source",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{
        CaptureLocalSnapshotCommand, capture_full_local_snapshot,
        create_preflighted_local_workspace, local_workspace_command,
    };
    use contexthub_domain::{EventId, WorkspaceId};
    use contexthub_workspace_fs::WorkspaceCancellationToken;
    use std::fs;
    use std::path::PathBuf;

    struct Fixture {
        parent: PathBuf,
        workspace: PathBuf,
        database: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let parent =
                std::env::temp_dir().join(format!("cilai-search-{}", uuid::Uuid::new_v4()));
            let workspace = parent.join("workspace");
            fs::create_dir_all(&workspace).unwrap();
            fs::create_dir(parent.join("data")).unwrap();
            Self {
                database: parent.join("data/search.db"),
                parent,
                workspace,
            }
        }

        fn snapshot(&self) -> SqlitePersistence {
            let mut persistence = SqlitePersistence::open(&self.database, "0.1.0").unwrap();
            create_preflighted_local_workspace(
                &mut persistence,
                &self.workspace,
                &[self.parent.join("data")],
                local_workspace_command(
                    "w1",
                    "Workspace",
                    "scan",
                    "created",
                    "create",
                    "desktop",
                    1,
                ),
            )
            .unwrap();
            capture_full_local_snapshot(
                &mut persistence,
                &CaptureLocalSnapshotCommand {
                    workspace_id: WorkspaceId::new("w1"),
                    snapshot_id: "snapshot".to_owned(),
                    event_id: EventId::new("snapshot-event"),
                    correlation_id: "snapshot-correlation".to_owned(),
                    observed_at: 10,
                    completed_at: 11,
                },
                &WorkspaceCancellationToken::new(),
            )
            .unwrap();
            persistence
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.parent).unwrap();
        }
    }

    fn search(persistence: &mut SqlitePersistence, value: &str) -> GlobalSearchResult {
        query(
            persistence,
            &GlobalSearchParams {
                query: value.to_owned(),
                limit: 100,
            },
        )
        .unwrap()
    }

    #[test]
    fn body_hits_require_current_bytes_without_waiting_for_another_scan() {
        let fixture = Fixture::new();
        let file = fixture.workspace.join("可见名称.md");
        let old = "旧正文关键词";
        let new = "新正文替代词";
        assert_eq!(old.len(), new.len());
        fs::write(&file, old).unwrap();
        let mut persistence = fixture.snapshot();
        let hits = search(&mut persistence, old);
        assert_eq!(hits.total, 1);
        assert!(hits.items[0].subtitle.contains(old));

        fs::write(&file, new).unwrap();
        assert!(search(&mut persistence, old).items.is_empty());
        let names = search(&mut persistence, "可见名称");
        assert_eq!(names.total, 1);
        assert_eq!(names.items[0].subtitle, "Workspace");

        fs::remove_file(file).unwrap();
        assert!(search(&mut persistence, old).items.is_empty());
        assert!(search(&mut persistence, "可见名称").items.is_empty());
    }

    #[test]
    fn replaced_file_identity_cannot_reuse_an_indexed_hit_even_with_identical_bytes() {
        let fixture = Fixture::new();
        let file = fixture.workspace.join("身份名称.md");
        let replacement = fixture.workspace.join("replacement.md");
        fs::write(&file, "身份正文关键词").unwrap();
        let mut persistence = fixture.snapshot();
        fs::write(&replacement, "身份正文关键词").unwrap();
        fs::remove_file(&file).unwrap();
        fs::rename(replacement, file).unwrap();
        assert!(search(&mut persistence, "身份正文关键词").items.is_empty());
        assert!(search(&mut persistence, "身份名称").items.is_empty());
    }

    #[test]
    fn replaced_root_identity_is_rejected_even_when_file_identity_and_bytes_survive() {
        let fixture = Fixture::new();
        fs::write(fixture.workspace.join("根目录名称.md"), "根目录正文关键词").unwrap();
        let mut persistence = fixture.snapshot();
        let original = fixture.parent.join("original");
        fs::rename(&fixture.workspace, &original).unwrap();
        fs::create_dir(&fixture.workspace).unwrap();
        fs::hard_link(
            original.join("根目录名称.md"),
            fixture.workspace.join("根目录名称.md"),
        )
        .unwrap();
        let path = WorkspaceRelativePath::parse("根目录名称.md").unwrap();
        assert_eq!(
            inspect_workspace_entry(&original, &path)
                .unwrap()
                .file_identity,
            inspect_workspace_entry(&fixture.workspace, &path)
                .unwrap()
                .file_identity
        );
        assert!(
            search(&mut persistence, "根目录正文关键词")
                .items
                .is_empty()
        );
        assert!(search(&mut persistence, "根目录名称").items.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn redirected_root_is_rejected_even_when_root_and_file_identities_survive() {
        let fixture = Fixture::new();
        fs::write(fixture.workspace.join("根目录名称.md"), "根目录正文关键词").unwrap();
        let mut persistence = fixture.snapshot();
        let original_identity = inspect_local_workspace_root(&fixture.workspace, &[])
            .unwrap()
            .local_root_identity;
        let moved = fixture.parent.join("moved");
        fs::rename(&fixture.workspace, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &fixture.workspace).unwrap();
        assert_eq!(
            inspect_local_workspace_root(&fixture.workspace, &[])
                .unwrap()
                .local_root_identity,
            original_identity
        );
        assert_eq!(
            fs::read_to_string(fixture.workspace.join("根目录名称.md")).unwrap(),
            "根目录正文关键词"
        );
        assert!(
            search(&mut persistence, "根目录正文关键词")
                .items
                .is_empty()
        );
        assert!(search(&mut persistence, "根目录名称").items.is_empty());
    }

    #[test]
    fn stale_candidates_do_not_end_pages_or_skip_later_current_hits() {
        let fixture = Fixture::new();
        for index in 0..70 {
            fs::write(
                fixture.workspace.join(format!("file-{index:02}.md")),
                "分页正文关键词",
            )
            .unwrap();
        }
        let mut persistence = fixture.snapshot();
        let connection = rusqlite::Connection::open(&fixture.database).unwrap();
        for index in 0..70 {
            connection
                .execute(
                    "UPDATE local_files SET updated_at=?1 WHERE relative_path=?2",
                    rusqlite::params![1000 - index, format!("file-{index:02}.md")],
                )
                .unwrap();
            if index < 64 {
                fs::remove_file(fixture.workspace.join(format!("file-{index:02}.md"))).unwrap();
            }
        }
        drop(connection);
        for value in ["分页正文关键词", "file-"] {
            let mut after = None;
            let mut paths = Vec::new();
            for page_number in 0..3 {
                let (page, next) = query_page(
                    &mut persistence,
                    &GlobalSearchParams {
                        query: value.to_owned(),
                        limit: 2,
                    },
                    after,
                )
                .unwrap();
                assert_eq!(page.items.len(), 2);
                assert_eq!(next.is_some(), page_number < 2);
                paths.extend(
                    page.items
                        .into_iter()
                        .map(|item| item.relative_path.unwrap()),
                );
                after = next;
            }
            assert_eq!(
                paths,
                (64..70)
                    .map(|index| format!("file-{index:02}.md"))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_and_parent_links_never_return_indexed_body_or_name_hits() {
        let fixture = Fixture::new();
        let directory = fixture.workspace.join("docs");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("linked-note.md"), "链接正文关键词").unwrap();
        fs::write(fixture.workspace.join("linked-direct.md"), "链接正文关键词").unwrap();
        let mut persistence = fixture.snapshot();
        let outside = fixture.parent.join("outside");
        fs::rename(&directory, &outside).unwrap();
        fs::remove_file(fixture.workspace.join("linked-direct.md")).unwrap();
        std::os::unix::fs::symlink(&outside, &directory).unwrap();
        std::os::unix::fs::symlink(
            outside.join("linked-note.md"),
            fixture.workspace.join("linked-direct.md"),
        )
        .unwrap();
        assert!(search(&mut persistence, "链接正文关键词").items.is_empty());
        assert!(search(&mut persistence, "linked-").items.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_body_and_uninspectable_name_report_incomplete_search() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let directory = fixture.workspace.join("protected");
        fs::create_dir(&directory).unwrap();
        let file = directory.join("权限名称.md");
        fs::write(&file, "权限正文关键词").unwrap();
        let mut persistence = fixture.snapshot();
        let permissions = fs::metadata(&file).unwrap().permissions();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o0)).unwrap();
        let denied = fs::File::open(&file);
        if denied.is_ok() {
            fs::set_permissions(&file, permissions).unwrap();
            eprintln!("permission fixture unavailable: process can read mode-000 files");
            return;
        }
        assert_eq!(denied.unwrap_err().kind(), ErrorKind::PermissionDenied);
        let result = query(
            &mut persistence,
            &GlobalSearchParams {
                query: "权限正文关键词".to_owned(),
                limit: 10,
            },
        );
        fs::set_permissions(&file, permissions).unwrap();
        assert_eq!(
            result.unwrap_err().reason(),
            Some("global_search_incomplete")
        );

        let permissions = fs::metadata(&directory).unwrap().permissions();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o0)).unwrap();
        let result = query(
            &mut persistence,
            &GlobalSearchParams {
                query: "权限名称".to_owned(),
                limit: 10,
            },
        );
        fs::set_permissions(&directory, permissions).unwrap();
        assert_eq!(
            result.unwrap_err().reason(),
            Some("global_search_incomplete")
        );
    }
}
