use crate::workspace::{
    CaptureLocalSnapshotCommand, capture_full_local_snapshot, create_preflighted_local_workspace,
    local_workspace_command,
};
use contexthub_domain::{EventId, WorkspaceId};
use contexthub_protocol::PersonalRestoreWorkspacePlanParams;
use contexthub_workspace_fs::WorkspaceCancellationToken;

fn indexed_originals(fixture: &mut Fixture) -> PathBuf {
    let source = fixture
        .root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("original-project");
    secure_dir(&source.join("notes")).unwrap();
    secure_dir(&source.join("empty")).unwrap();
    fs::write(source.join("notes/design.txt"), b"indexed original bytes").unwrap();
    let db = fixture.services.persistence.as_mut().unwrap();
    create_preflighted_local_workspace(
        db,
        &source,
        std::slice::from_ref(&fixture.profile),
        local_workspace_command(
            "original-workspace",
            "Original project",
            "scan-original",
            "create-original",
            "original-correlation",
            "desktop",
            10,
        ),
    )
    .unwrap();
    capture_full_local_snapshot(
        db,
        &CaptureLocalSnapshotCommand {
            workspace_id: WorkspaceId::new("original-workspace"),
            snapshot_id: "complete-original-snapshot".into(),
            event_id: EventId::new("complete-original-event"),
            correlation_id: "original-correlation".into(),
            observed_at: 11,
            completed_at: 12,
        },
        &WorkspaceCancellationToken::new(),
    )
    .unwrap();
    source
}
fn prepare_originals(fixture: &mut Fixture) -> PersonalDataOperation {
    fixture
        .services
        .data_backup_prepare(
            &fixture.profile,
            &fixture.id,
            PersonalBackupPrepareParams {
                selection: PersonalBackupSelection {
                    include_workspace_files: vec!["original-workspace".into()],
                },
                client_operation_id: uuid::Uuid::new_v4().to_string(),
            },
            20,
        )
        .unwrap()
}
fn backup_originals(fixture: &mut Fixture) -> PersonalDataOperation {
    let operation = prepare_originals(fixture);
    let proof = fixture.proof(&operation);
    let result = fixture.capture(&operation, Some(proof.clone()));
    fixture.release(&proof);
    result
}
fn prepare_original_restore(
    fixture: &mut Fixture,
    backup: &PersonalDataOperation,
) -> PersonalDataOperation {
    let archive = operation_root(&fixture.profile, &backup.operation_id)
        .unwrap()
        .join("backup.zip");
    fixture
        .services
        .data_restore_prepare(
            &fixture.profile,
            &fixture.id,
            &PersonalRestorePrepareParams {
                archive_path: archive.to_string_lossy().into_owned(),
                client_operation_id: uuid::Uuid::new_v4().to_string(),
            },
            30,
        )
        .unwrap()
}
fn plan_original_restore(
    fixture: &mut Fixture,
    restore: &PersonalDataOperation,
) -> PersonalDataOperation {
    let destination = fixture
        .root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("restore-destination");
    secure_dir(&destination).unwrap();
    fixture
        .services
        .data_restore_workspace_plan(
            &fixture.profile,
            &fixture.id,
            &PersonalRestoreWorkspacePlanParams {
                operation_id: restore.operation_id.clone(),
                preview_digest: restore.preview_digest.clone(),
                destination_parent: destination.to_string_lossy().into_owned(),
                client_operation_id: "native-picker-plan".into(),
            },
            31,
        )
        .unwrap()
}
fn ready_original_restore(
    fixture: &mut Fixture,
    restore: &PersonalDataOperation,
) -> PersonalDataOperation {
    let proof = fixture.proof(restore);
    fixture
        .services
        .data_restore_apply_prepare(
            &fixture.profile,
            &fixture.id,
            &PersonalRestoreApplyPrepareParams {
                operation_id: restore.operation_id.clone(),
                preview_digest: restore.preview_digest.clone(),
                expected_current_profile_id: fixture.id.clone(),
                client_operation_id: "explicit-restore".into(),
                quiescence: Some(proof),
            },
            "test-instance",
            32,
        )
        .unwrap()
}

#[test]
fn workspace_originals_capture_matches_frozen_index_and_declares_ignored_material() {
    let mut fixture = Fixture::new();
    let source = indexed_originals(&mut fixture);
    fs::create_dir(source.join("node_modules")).unwrap();
    fs::write(
        source.join("node_modules/private.txt"),
        b"rebuildable excluded material",
    )
    .unwrap();
    let result = backup_originals(&mut fixture);
    assert_eq!(
        result.state,
        PersonalDataOperationState::Completed,
        "{:?}",
        result.blockers
    );
    let bytes = fs::read(
        operation_root(&fixture.profile, &result.operation_id)
            .unwrap()
            .join("backup.zip"),
    )
    .unwrap();
    let (manifest, files) = validate_archive(&bytes).unwrap();
    assert_eq!(manifest.format_version, 2);
    assert!(manifest.workspaces[0].original_files_included);
    assert!(
        manifest.workspaces[0]
            .content_snapshot
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.relative_path == "empty" && entry.file_type == "directory")
    );
    assert_eq!(
        files[&format!(
            "{}notes/design.txt",
            workspaces::prefix("original-workspace")
        )]
            .1,
        b"indexed original bytes"
    );
    assert!(
        manifest
            .exclusions
            .iter()
            .any(|item| item.code == "workspace_index_policy_excluded"
                && item.relative_path.as_deref() == Some("original-workspace/node_modules"))
    );
    assert!(!files.keys().any(|path| path.contains("node_modules")));
}

#[test]
fn workspace_originals_changed_or_unindexed_members_are_persistently_incomplete() {
    for scenario in ["changed", "extra", "removed", "replaced"] {
        let mut fixture = Fixture::new();
        let source = indexed_originals(&mut fixture);
        let operation = prepare_originals(&mut fixture);
        match scenario {
            "changed" => fs::write(
                source.join("notes/design.txt"),
                b"different bytes after confirmation",
            )
            .unwrap(),
            "extra" => fs::write(source.join("unindexed.txt"), b"new file").unwrap(),
            "removed" => fs::remove_file(source.join("notes/design.txt")).unwrap(),
            "replaced" => {
                let path = source.join("notes/design.txt");
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                fs::rename(
                    &path,
                    source.parent().unwrap().join("retained-original.txt"),
                )
                .unwrap();
                fs::write(&path, b"indexed original bytes").unwrap();
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(modified))
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let proof = fixture.proof(&operation);
        let result = fixture.capture(&operation, Some(proof.clone()));
        assert_eq!(
            result.state,
            PersonalDataOperationState::Incomplete,
            "{scenario}"
        );
        assert!(
            result
                .result
                .as_ref()
                .unwrap()
                .artifact_relative_path
                .is_none()
        );
        assert!(
            !operation_root(&fixture.profile, &operation.operation_id)
                .unwrap()
                .join("backup.zip")
                .exists()
        );
        fixture.release(&proof);
    }
}

#[test]
fn workspace_originals_restore_requires_explicit_safe_new_destination() {
    let mut fixture = Fixture::new();
    indexed_originals(&mut fixture);
    let backup = backup_originals(&mut fixture);
    let restore = prepare_original_restore(&mut fixture, &backup);
    let proof = fixture.proof(&restore);
    let failed = fixture
        .services
        .data_restore_apply_prepare(
            &fixture.profile,
            &fixture.id,
            &PersonalRestoreApplyPrepareParams {
                operation_id: restore.operation_id.clone(),
                preview_digest: restore.preview_digest.clone(),
                expected_current_profile_id: fixture.id.clone(),
                client_operation_id: "without-destination".into(),
                quiescence: Some(proof.clone()),
            },
            "test-instance",
            31,
        )
        .unwrap_err();
    assert_eq!(
        failed.reason,
        "personal_data_workspace_destination_required"
    );
    fixture.release(&proof);
    let rejected = fixture
        .services
        .data_restore_workspace_plan(
            &fixture.profile,
            &fixture.id,
            &PersonalRestoreWorkspacePlanParams {
                operation_id: restore.operation_id.clone(),
                preview_digest: restore.preview_digest.clone(),
                destination_parent: fixture.profile.to_string_lossy().into_owned(),
                client_operation_id: "protected-destination".into(),
            },
            31,
        )
        .unwrap_err();
    assert_eq!(
        rejected.reason,
        "personal_data_workspace_destination_protected"
    );
    let planned = plan_original_restore(&mut fixture, &restore);
    assert_ne!(planned.preview_digest, restore.preview_digest);
    assert!(
        planned
            .restore_preview
            .unwrap()
            .workspace_destination
            .is_some()
    );
}

#[test]
fn workspace_originals_offline_restore_uses_new_usable_root_and_public_relocation() {
    let mut fixture = Fixture::new();
    let source = indexed_originals(&mut fixture);
    let backup = backup_originals(&mut fixture);
    assert_eq!(
        backup.state,
        PersonalDataOperationState::Completed,
        "{:?}",
        backup.blockers
    );
    let restore = prepare_original_restore(&mut fixture, &backup);
    let planned = plan_original_restore(&mut fixture, &restore);
    let ready = ready_original_restore(&mut fixture, &planned);
    fixture.services.persistence.take();
    let restored =
        offline_restore(&fixture.root, &ready.operation_id, &ready.preview_digest).unwrap();
    let result = restored.result.unwrap();
    let location = &result.restored_workspaces.as_ref().unwrap()[0];
    let target = Path::new(&location.restored_path);
    assert_eq!(
        fs::read(source.join("notes/design.txt")).unwrap(),
        b"indexed original bytes"
    );
    assert_eq!(
        fs::read(target.join("notes/design.txt")).unwrap(),
        b"indexed original bytes"
    );
    assert!(target.join("empty").is_dir());
    assert!(!target.starts_with(&fixture.root));
    let new_profile = fixture
        .root
        .join("profiles")
        .join(result.restored_profile_id.unwrap());
    let mut db = contexthub_persistence_sqlite::SqlitePersistence::open(
        &new_profile.join("contexthub.db"),
        "test",
    )
    .unwrap();
    let before = db
        .transact(|repository| repository.workspace(&WorkspaceId::new("original-workspace")))
        .unwrap()
        .unwrap();
    assert_eq!(before.local_root, location.restored_path);
    assert_eq!(
        before.local_status,
        contexthub_domain::LocalWorkspaceStatus::Ready
    );
    contexthub_workspace_fs::inspect_local_workspace_root(
        target,
        std::slice::from_ref(&new_profile),
    )
    .unwrap();
    let moved = target.with_file_name("explicitly-relocated-project");
    fs::rename(target, &moved).unwrap();
    let relocated = create_preflighted_local_workspace(
        &mut db,
        &moved,
        std::slice::from_ref(&new_profile),
        local_workspace_command(
            "must-not-create-new-id",
            "Relocated",
            "relocation-task",
            "relocation-event",
            "relocation-correlation",
            "desktop",
            40,
        ),
    )
    .unwrap();
    assert_eq!(
        relocated.business.workspace_id.as_str(),
        "original-workspace"
    );
    assert_eq!(Path::new(&relocated.business.local_root), moved);
    assert_eq!(
        relocated.business.local_root_identity,
        before.local_root_identity
    );
    assert_eq!(
        db.transact(|repository| repository.workspaces())
            .unwrap()
            .len(),
        1
    );
    assert!(fixture.profile.join("contexthub.db").exists());
}

#[test]
fn workspace_originals_destination_collision_keeps_manifest_and_existing_files() {
    let mut fixture = Fixture::new();
    indexed_originals(&mut fixture);
    let backup = backup_originals(&mut fixture);
    let restore = prepare_original_restore(&mut fixture, &backup);
    let planned = plan_original_restore(&mut fixture, &restore);
    let ready = ready_original_restore(&mut fixture, &planned);
    let original_manifest = fs::read(fixture.root.join("local-profile.json")).unwrap();
    let target = PathBuf::from(
        &ready
            .restore_preview
            .as_ref()
            .unwrap()
            .workspace_destination
            .as_ref()
            .unwrap()
            .target_path,
    );
    secure_dir(&target).unwrap();
    fs::write(target.join("keep.txt"), b"unrelated new existing data").unwrap();
    fixture.services.persistence.take();
    assert_eq!(
        offline_restore(&fixture.root, &ready.operation_id, &ready.preview_digest)
            .unwrap_err()
            .reason,
        "personal_data_restore_target_exists"
    );
    assert_eq!(
        fs::read(target.join("keep.txt")).unwrap(),
        b"unrelated new existing data"
    );
    assert_eq!(
        fs::read(fixture.root.join("local-profile.json")).unwrap(),
        original_manifest
    );
}

#[test]
fn workspace_originals_continuous_external_write_is_incomplete_without_export() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let mut fixture = Fixture::new();
    let source = indexed_originals(&mut fixture);
    let operation = prepare_originals(&mut fixture);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let path = source.join("notes/design.txt");
    let (started, ready) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        fs::write(&path, b"ongoing external edit A").unwrap();
        started.send(()).unwrap();
        while !thread_stop.load(Ordering::SeqCst) {
            fs::write(&path, b"ongoing external edit B").unwrap();
            std::thread::yield_now();
        }
    });
    ready.recv().unwrap();
    let proof = fixture.proof(&operation);
    let result = fixture.capture(&operation, Some(proof.clone()));
    stop.store(true, Ordering::SeqCst);
    writer.join().unwrap();
    assert_eq!(result.state, PersonalDataOperationState::Incomplete);
    assert!(
        result
            .result
            .as_ref()
            .unwrap()
            .artifact_relative_path
            .is_none()
    );
    assert!(
        !operation_root(&fixture.profile, &result.operation_id)
            .unwrap()
            .join("backup.zip")
            .exists()
    );
    fixture.release(&proof);
}

#[test]
fn workspace_originals_new_index_generation_cannot_replace_confirmed_basis() {
    let mut fixture = Fixture::new();
    indexed_originals(&mut fixture);
    let operation = prepare_originals(&mut fixture);
    capture_full_local_snapshot(
        fixture.services.persistence.as_mut().unwrap(),
        &CaptureLocalSnapshotCommand {
            workspace_id: WorkspaceId::new("original-workspace"),
            snapshot_id: "later-full-generation".into(),
            event_id: EventId::new("later-scan-event"),
            correlation_id: "later-correlation".into(),
            observed_at: 21,
            completed_at: 22,
        },
        &WorkspaceCancellationToken::new(),
    )
    .unwrap();
    let proof = fixture.proof(&operation);
    let result = fixture.capture(&operation, Some(proof.clone()));
    assert_eq!(result.state, PersonalDataOperationState::Incomplete);
    assert_eq!(
        result.blockers[0].code,
        "personal_data_workspace_index_changed"
    );
    assert!(
        !operation_root(&fixture.profile, &result.operation_id)
            .unwrap()
            .join("backup.zip")
            .exists()
    );
    fixture.release(&proof);
}

#[test]
fn workspace_originals_missing_full_scan_or_strong_hash_cannot_prepare() {
    let mut fixture = Fixture::new();
    let source = fixture
        .root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("unscanned-project");
    secure_dir(&source).unwrap();
    create_preflighted_local_workspace(
        fixture.services.persistence.as_mut().unwrap(),
        &source,
        std::slice::from_ref(&fixture.profile),
        local_workspace_command(
            "unscanned",
            "Unscanned",
            "unscanned-task",
            "unscanned-event",
            "unscanned-correlation",
            "desktop",
            1,
        ),
    )
    .unwrap();
    let error = fixture
        .services
        .data_backup_prepare(
            &fixture.profile,
            &fixture.id,
            PersonalBackupPrepareParams {
                selection: PersonalBackupSelection {
                    include_workspace_files: vec!["unscanned".into()],
                },
                client_operation_id: "missing-full-index".into(),
            },
            2,
        )
        .unwrap_err();
    assert!(matches!(
        error.reason,
        "personal_data_workspace_index_unready" | "personal_data_workspace_full_index_required"
    ));
    indexed_originals(&mut fixture);
    let db = rusqlite::Connection::open(fixture.profile.join("contexthub.db")).unwrap();
    db.execute("UPDATE local_files SET local_checksum_algorithm=NULL,local_checksum_value=NULL WHERE workspace_id='original-workspace' AND file_type='file'", []).unwrap();
    drop(db);
    let error = fixture
        .services
        .data_backup_prepare(
            &fixture.profile,
            &fixture.id,
            PersonalBackupPrepareParams {
                selection: PersonalBackupSelection {
                    include_workspace_files: vec!["original-workspace".into()],
                },
                client_operation_id: "missing-strong-hash".into(),
            },
            20,
        )
        .unwrap_err();
    assert_eq!(error.reason, "personal_data_workspace_hash_missing");
}

#[test]
fn workspace_originals_archive_membership_and_database_basis_are_both_validated() {
    let mut fixture = Fixture::new();
    indexed_originals(&mut fixture);
    let backup = backup_originals(&mut fixture);
    let bytes = fs::read(
        operation_root(&fixture.profile, &backup.operation_id)
            .unwrap()
            .join("backup.zip"),
    )
    .unwrap();
    let (manifest, files) = validate_archive(&bytes).unwrap();
    let root = fixture.root.parent().unwrap().parent().unwrap();
    let mut invalid_members = manifest.clone();
    invalid_members.workspaces[0]
        .content_snapshot
        .as_mut()
        .unwrap()
        .entries
        .clear();
    let content = invalid_members.workspaces[0]
        .content_snapshot
        .as_mut()
        .unwrap();
    content.digest.clear();
    content.digest = fingerprint(content).unwrap();
    let invalid = root.join("invalid-members.zip");
    write_archive(&invalid, &invalid_members, &files).unwrap();
    assert!(validate_archive(&fs::read(&invalid).unwrap()).is_err());
    let mut invalid_basis = manifest;
    invalid_basis.workspaces[0]
        .content_snapshot
        .as_mut()
        .unwrap()
        .workspace_revision += 1;
    let content = invalid_basis.workspaces[0]
        .content_snapshot
        .as_mut()
        .unwrap();
    content.digest.clear();
    content.digest = fingerprint(content).unwrap();
    let invalid = root.join("invalid-basis.zip");
    write_archive(&invalid, &invalid_basis, &files).unwrap();
    assert!(validate_archive(&fs::read(&invalid).unwrap()).is_ok());
    let original_manifest = fs::read(fixture.root.join("local-profile.json")).unwrap();
    let rejected = fixture
        .services
        .data_restore_prepare(
            &fixture.profile,
            &fixture.id,
            &PersonalRestorePrepareParams {
                archive_path: invalid.to_string_lossy().into_owned(),
                client_operation_id: "invalid-basis".into(),
            },
            30,
        )
        .unwrap_err();
    assert_eq!(rejected.reason, "personal_data_workspace_index_changed");
    assert_eq!(
        fs::read(fixture.root.join("local-profile.json")).unwrap(),
        original_manifest
    );
}

#[test]
fn workspace_originals_version_one_index_only_archive_remains_readable() {
    let mut fixture = Fixture::new();
    let operation = fixture.prepare();
    let proof = fixture.proof(&operation);
    let result = fixture.capture(&operation, Some(proof.clone()));
    fixture.release(&proof);
    let bytes = fs::read(
        operation_root(&fixture.profile, &result.operation_id)
            .unwrap()
            .join("backup.zip"),
    )
    .unwrap();
    let (mut manifest, files) = validate_archive(&bytes).unwrap();
    manifest.format_version = 1;
    let old = fixture
        .root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("old-index-only.zip");
    write_archive(&old, &manifest, &files).unwrap();
    assert_eq!(
        validate_archive(&fs::read(old).unwrap())
            .unwrap()
            .0
            .format_version,
        1
    );
}

#[test]
fn workspace_originals_changed_destination_parent_identity_keeps_old_profile() {
    let mut fixture = Fixture::new();
    indexed_originals(&mut fixture);
    let backup = backup_originals(&mut fixture);
    let restore = prepare_original_restore(&mut fixture, &backup);
    let planned = plan_original_restore(&mut fixture, &restore);
    let ready = ready_original_restore(&mut fixture, &planned);
    let parent = PathBuf::from(
        &ready
            .restore_preview
            .as_ref()
            .unwrap()
            .workspace_destination
            .as_ref()
            .unwrap()
            .parent_path,
    );
    fs::rename(
        &parent,
        parent.with_file_name("retained-destination-parent"),
    )
    .unwrap();
    secure_dir(&parent).unwrap();
    let original_manifest = fs::read(fixture.root.join("local-profile.json")).unwrap();
    fixture.services.persistence.take();
    assert_eq!(
        offline_restore(&fixture.root, &ready.operation_id, &ready.preview_digest)
            .unwrap_err()
            .reason,
        "personal_data_workspace_destination_changed"
    );
    assert_eq!(
        fs::read(fixture.root.join("local-profile.json")).unwrap(),
        original_manifest
    );
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
}
