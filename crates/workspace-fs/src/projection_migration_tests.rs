use super::{tests::encoded, tests::test_root, *};

fn legacy_projection(root: &Path, manifests: &EncodedManifestSet) -> PathBuf {
    let legacy = root.join(LEGACY_WORKSPACE_METADATA_DIRECTORY);
    prepare_projection_directory(&legacy).unwrap();
    for (name, bytes) in [
        (WORKSPACE_FILE, &manifests.workspace_json),
        (BASELINE_FILE, &manifests.baseline_json),
        (CONFIRMATION_FILE, &manifests.confirmation_json),
        (EXPORT_STATE_FILE, &manifests.export_state_json),
    ] {
        atomic_write(&legacy, name, bytes).unwrap();
    }
    legacy
}

fn expected(manifests: &EncodedManifestSet) -> ExpectedProjectionState {
    ExpectedProjectionState {
        exported_revision: manifests.database_export_revision,
        manifest_checksum: Some(manifests.manifest_checksum.clone()),
        allow_incomplete_recovery: false,
    }
}

#[test]
fn migrates_current_or_pending_projection_and_preserves_workspace_and_extra_files() {
    for revision in [1, 2] {
        let root = test_root("metadata-migration");
        let original = encoded(1, 10);
        let legacy = legacy_projection(&root, &original);
        fs::write(root.join("user.md"), b"user content").unwrap();
        fs::create_dir(legacy.join("cloud-pull")).unwrap();
        fs::write(legacy.join("cloud-pull/incomplete.part"), b"partial").unwrap();
        let incoming = encoded(revision, 20);
        let result = write_projection(&root, &incoming, &expected(&original)).unwrap();
        assert_eq!(
            result,
            if revision == 1 {
                ProjectionWriteOutcome::AlreadyCurrent
            } else {
                ProjectionWriteOutcome::Exported
            }
        );
        assert!(!legacy.exists());
        let current = root.join(WORKSPACE_METADATA_DIRECTORY);
        assert_eq!(
            fs::read(current.join(WORKSPACE_FILE)).unwrap(),
            incoming.workspace_json
        );
        assert_eq!(
            fs::read(current.join("cloud-pull/incomplete.part")).unwrap(),
            b"partial"
        );
        assert_eq!(fs::read(root.join("user.md")).unwrap(), b"user content");
        assert_eq!(
            write_projection(&root, &incoming, &expected(&incoming)).unwrap(),
            ProjectionWriteOutcome::AlreadyCurrent
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn simultaneous_metadata_directories_are_preserved_as_an_explicit_conflict() {
    let root = test_root("metadata-dual");
    let manifests = encoded(1, 10);
    let legacy = legacy_projection(&root, &manifests);
    let current = root.join(WORKSPACE_METADATA_DIRECTORY);
    fs::create_dir(&current).unwrap();
    fs::write(current.join("keep.txt"), b"existing cilai data").unwrap();
    let error = write_projection(&root, &manifests, &expected(&manifests)).unwrap_err();
    assert_eq!(
        error.kind(),
        ProjectionWriteErrorKind::ExistingProjectionConflict
    );
    assert_eq!(
        fs::read(legacy.join(WORKSPACE_FILE)).unwrap(),
        manifests.workspace_json
    );
    assert_eq!(
        fs::read(current.join("keep.txt")).unwrap(),
        b"existing cilai data"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_or_unowned_legacy_projection_is_not_migrated_or_overwritten() {
    for mutation in ["checksum", "workspace", "state"] {
        let root = test_root(mutation);
        let manifests = encoded(1, 10);
        let legacy = legacy_projection(&root, &manifests);
        let changed_path = match mutation {
            "checksum" => legacy.join(BASELINE_FILE),
            "workspace" => legacy.join(WORKSPACE_FILE),
            _ => legacy.join(EXPORT_STATE_FILE),
        };
        let changed = if mutation == "workspace" {
            let mut workspace: serde_json::Value =
                serde_json::from_slice(&manifests.workspace_json).unwrap();
            workspace["workspace_id"] = "another-app-workspace".into();
            serde_json::to_vec(&workspace).unwrap()
        } else {
            b"corrupt".to_vec()
        };
        fs::write(&changed_path, &changed).unwrap();
        assert!(write_projection(&root, &manifests, &expected(&manifests)).is_err());
        assert_eq!(fs::read(&changed_path).unwrap(), changed);
        assert!(legacy.is_dir());
        assert!(!root.join(WORKSPACE_METADATA_DIRECTORY).exists());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn legacy_file_or_unrecognized_directory_is_never_removed() {
    for directory in [false, true] {
        let root = test_root("metadata-unrecognized");
        let legacy = root.join(LEGACY_WORKSPACE_METADATA_DIRECTORY);
        if directory {
            fs::create_dir(&legacy).unwrap();
            fs::write(legacy.join("keep.txt"), b"unrecognized data").unwrap();
        } else {
            fs::write(&legacy, b"unrecognized data").unwrap();
        }
        let manifests = encoded(1, 10);
        assert!(write_projection(&root, &manifests, &expected(&manifests)).is_err());
        assert_eq!(
            fs::read(if directory {
                legacy.join("keep.txt")
            } else {
                legacy
            })
            .unwrap(),
            b"unrecognized data"
        );
        assert!(!root.join(WORKSPACE_METADATA_DIRECTORY).exists());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn incomplete_legacy_projection_requires_explicit_recovery_and_workspace_identity() {
    let root = test_root("metadata-incomplete");
    let manifests = encoded(1, 10);
    let legacy = root.join(LEGACY_WORKSPACE_METADATA_DIRECTORY);
    prepare_projection_directory(&legacy).unwrap();
    atomic_write(&legacy, WORKSPACE_FILE, &manifests.workspace_json).unwrap();
    let mut expectation = ExpectedProjectionState {
        exported_revision: 0,
        manifest_checksum: None,
        allow_incomplete_recovery: false,
    };
    assert!(write_projection(&root, &manifests, &expectation).is_err());
    assert!(legacy.is_dir());
    expectation.allow_incomplete_recovery = true;
    assert_eq!(
        write_projection(&root, &manifests, &expectation).unwrap(),
        ProjectionWriteOutcome::Exported
    );
    assert!(!legacy.exists());
    assert_eq!(
        fs::read(
            root.join(WORKSPACE_METADATA_DIRECTORY)
                .join(EXPORT_STATE_FILE)
        )
        .unwrap(),
        manifests.export_state_json
    );
    fs::remove_dir_all(root).unwrap();
}
