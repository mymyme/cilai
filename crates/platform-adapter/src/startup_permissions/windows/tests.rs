use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture {
    base: PathBuf,
    request: Request,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "cilai-startup-repair-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let mut plan = RepairPlan {
            local_data: base.to_string_lossy().into_owned(),
            environment: "business".into(),
            profile_id: "0123456789abcdef0123456789abcdef".into(),
            user_sid: user_sid(&current_token().unwrap()).unwrap(),
            parent_pid: std::process::id(),
            parent_fingerprint: crate::instance::process_start_fingerprint(std::process::id())
                .unwrap()
                .unwrap(),
            expires_at: now() + 300,
            anchors: vec![],
        };
        let profile = profile(&plan);
        for directory in [
            profile.join("run"),
            profile.join("ai-runtime/cilai-harness/.sandbox"),
            root(&plan).join("profiles"),
        ] {
            std::fs::create_dir_all(directory).unwrap();
        }
        crate::security::ensure_restricted_directory(&layout::recovery(&base)).unwrap();
        for (name, contents) in [
            ("run/instance.json", "original-instance"),
            ("run/instance.lock", ""),
            ("contexthub.db", "database-bytes-unchanged"),
            (
                "ai-runtime/cilai-harness/thread_history_1.sqlite",
                "history-bytes-unchanged",
            ),
            ("ai-runtime/cilai-harness/.sandbox/keep", "sandbox-data"),
        ] {
            std::fs::write(profile.join(name), contents).unwrap();
        }
        plan.anchors = anchors(&plan).unwrap();
        let backup_name = "12345678-1234-1234-1234-123456789abc.json".to_owned();
        let backup = layout::recovery(&base).join(&backup_name);
        let file = crate::security::create_new_restricted_file(&backup).unwrap();
        let mut request = Request {
            plan,
            backup_name,
            backup_identity: identity(&file).unwrap(),
            journal_identity: String::new(),
        };
        let journal_file =
            crate::security::create_new_restricted_file(&journal::path(&request).unwrap()).unwrap();
        request.journal_identity = identity(&journal_file).unwrap();
        Self { base, request }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}

#[test]
fn repairs_existing_startup_acl_preserves_bytes_sandbox_and_backup() {
    let fixture = Fixture::new();
    let profile = profile(&fixture.request.plan);
    let untouched = profile.join("ai-runtime/cilai-harness/.sandbox");
    let sandbox_before = snapshot(&open(&untouched, true, MAXIMUM_ALLOWED).unwrap()).unwrap();
    let original =
        snapshot(&open(&profile.join("run/instance.json"), false, MAXIMUM_ALLOWED).unwrap())
            .unwrap();
    apply(&fixture.request).unwrap();
    verify(&fixture.request.plan).unwrap();
    assert_eq!(
        std::fs::read_to_string(profile.join("contexthub.db")).unwrap(),
        "database-bytes-unchanged"
    );
    assert_eq!(
        std::fs::read_to_string(profile.join("ai-runtime/cilai-harness/thread_history_1.sqlite"))
            .unwrap(),
        "history-bytes-unchanged"
    );
    assert_eq!(
        std::fs::read_to_string(profile.join("run/instance.json")).unwrap(),
        "original-instance"
    );
    assert_eq!(
        snapshot(&open(&untouched, true, MAXIMUM_ALLOWED).unwrap()).unwrap(),
        sandbox_before
    );
    let backup = std::fs::read_to_string(backup_path(&fixture.request).unwrap()).unwrap();
    assert!(backup.contains(&original));
    assert!(!backup.contains("database-bytes-unchanged"));
    let before_repeat = snapshot(
        &open(
            &profile.join("ai-runtime/cilai-harness"),
            true,
            MAXIMUM_ALLOWED,
        )
        .unwrap(),
    )
    .unwrap();
    apply(&fixture.request).unwrap();
    assert_eq!(
        snapshot(
            &open(
                &profile.join("ai-runtime/cilai-harness"),
                true,
                MAXIMUM_ALLOWED
            )
            .unwrap()
        )
        .unwrap(),
        before_repeat
    );
}

#[test]
fn collected_targets_keep_all_ancestors_pinned_until_repair_finishes() {
    let fixture = Fixture::new();
    let targets = collect(&fixture.request.plan).unwrap();
    let moved = fixture.base.with_extension("moved");
    assert!(std::fs::rename(&fixture.base, &moved).is_err());
    drop(targets);
    std::fs::rename(&fixture.base, &moved).unwrap();
    std::fs::rename(&moved, &fixture.base).unwrap();
}

#[test]
fn recovers_access_denied_with_a_privileged_handle_then_verifies_ordinary_access() {
    let fixture = Fixture::new();
    let path = profile(&fixture.request.plan).join("run/instance.json");
    let handle = open(&path, false, MAXIMUM_ALLOWED).unwrap();
    let sid = &fixture.request.plan.user_sid;
    // Removing READ_CONTROL from OWNER RIGHTS reproduces the reported OS error 5.
    // Retain the pre-existing full-access handle to model the elevated helper's
    // privileged handle without triggering UAC or touching real application data.
    set_security(&handle, &format!("O:{sid}D:P(A;;WDWO;;;OW)(A;;FA;;;SY)")).unwrap();
    assert_eq!(
        crate::security::verify_current_user_private_file(&path)
            .unwrap_err()
            .raw_os_error(),
        Some(5)
    );
    set_security(&handle, &format!("O:{sid}{PRIVATE_DACL}")).unwrap();
    crate::security::verify_current_user_private_file(&path).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original-instance");
}

#[test]
fn busy_instance_and_hardlinks_are_rejected_before_any_acl_change() {
    let fixture = Fixture::new();
    let profile = profile(&fixture.request.plan);
    let lock = File::options()
        .read(true)
        .write(true)
        .open(profile.join("run/instance.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let before = snapshot(&open(&profile, true, MAXIMUM_ALLOWED).unwrap()).unwrap();
    assert_eq!(
        apply(&fixture.request).unwrap_err().raw_os_error(),
        Some(BUSY)
    );
    assert_eq!(
        snapshot(&open(&profile, true, MAXIMUM_ALLOWED).unwrap()).unwrap(),
        before
    );
    drop(lock);
    std::fs::hard_link(
        profile.join("run/instance.json"),
        fixture.base.join("outside.json"),
    )
    .unwrap();
    assert_eq!(
        apply(&fixture.request).unwrap_err().raw_os_error(),
        Some(UNSAFE_OBJECT)
    );
    assert_eq!(
        snapshot(&open(&profile, true, MAXIMUM_ALLOWED).unwrap()).unwrap(),
        before
    );
}

#[test]
fn changed_acl_rolls_back_previous_changes_without_overwriting_conflicting_acl() {
    let fixture = Fixture::new();
    let targets = collect(&fixture.request.plan).unwrap();
    let first_before = snapshot(&targets[0].file).unwrap();
    let sid = &fixture.request.plan.user_sid;
    set_security(&targets[1].file, &format!("O:{sid}{PRIVATE_DACL}")).unwrap();
    let conflicting = snapshot(&targets[1].file).unwrap();
    let mut journal = RepairJournal::new(&fixture.request).unwrap();
    assert_eq!(
        apply_changes(&targets, sid, &mut journal)
            .unwrap_err()
            .raw_os_error(),
        Some(Conflict::PermissionsChanged as i32)
    );
    assert_eq!(snapshot(&targets[0].file).unwrap(), first_before);
    assert_eq!(snapshot(&targets[1].file).unwrap(), conflicting);
    assert_eq!(journal.summary.target_index, Some(1));
    assert_eq!(journal.summary.previous_applied_index, Some(0));
    assert_eq!(journal.summary.phase, super::super::RepairPhase::Compare);
    assert_eq!(journal.summary.rollback_verified, Some(true));
    assert!(!journal.summary.changed_parts.is_empty());
    let mut file = journal::open_for_write(&fixture.request).unwrap();
    journal::save(&mut file, &journal).unwrap();
    drop(file);
    let exported = journal::recent(&fixture.request.plan).unwrap();
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].target_index, Some(1));
    assert_eq!(
        exported[0].os_error,
        Some(Conflict::PermissionsChanged as i32)
    );
    let serialized = serde_json::to_string(&exported).unwrap();
    for private in [
        sid.as_str(),
        conflicting.as_str(),
        targets[1].path.to_str().unwrap(),
    ] {
        assert!(!serialized.contains(private));
    }
}

fn seed_legacy_fixture_descriptor(path: &Path, sddl: &str) {
    let text = wide(OsStr::new(sddl));
    let mut descriptor = ptr::null_mut();
    // SAFETY: fixture-only descriptor, allocated by Windows and retained below.
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    })
    .unwrap();
    let _memory = LocalMemory(descriptor);
    let name = wide(path.as_os_str());
    // SAFETY: a newly created isolated fixture, never a real application path.
    // This old setter recreates the exact legacy DACL with no AI control flag.
    check(unsafe {
        windows_sys::Win32::Security::SetFileSecurityW(
            name.as_ptr(),
            DACL_SECURITY_INFORMATION,
            descriptor,
        )
    })
    .unwrap();
}

#[test]
fn replays_captured_inherited_source_change_through_native_repair_without_losing_data() {
    let fixture = Fixture::new();
    let plan = &fixture.request.plan;
    let child = root(plan);
    let original = snapshot(&open(&child, true, MAXIMUM_ALLOWED).unwrap()).unwrap();
    let inherited = format!(
        "{}D:(A;OICIID;FA;;;SY)(A;OICIID;FA;;;{})",
        &original[..original.find("D:").unwrap()],
        plan.user_sid
    );
    seed_legacy_fixture_descriptor(&child, &inherited);
    let targets = collect(plan).unwrap();
    let index = targets
        .iter()
        .position(|target| target.path == child)
        .unwrap();
    assert_eq!(targets[index].before, inherited);
    // Replay the observed comparison input. Local Win32 setters normalize this
    // transient legacy state differently, so this does NOT claim to reproduce
    // the failing machine's OS trigger. Writes and final readbacks stay native.
    let converted = inherited.replace("OICIID", "OICI");
    let sandbox = profile(plan).join("ai-runtime/cilai-harness/.sandbox");
    let sandbox_before = snapshot(&open(&sandbox, true, MAXIMUM_ALLOWED).unwrap()).unwrap();
    let mut journal = RepairJournal::new(&fixture.request).unwrap();
    let mut replayed = false;
    apply_changes_with_snapshot(&targets, &plan.user_sid, &mut journal, |file| {
        if file.as_raw_handle() == targets[index].file.as_raw_handle() {
            assert!(!replayed);
            replayed = true;
            assert!(snapshot(&targets[0].file)?.contains("D:P"));
            Ok(converted.clone())
        } else {
            snapshot(file)
        }
    })
    .unwrap();
    assert!(replayed);
    drop(targets);
    verify(plan).unwrap();
    assert_eq!(
        serde_json::to_value(journal).unwrap()["inheritedSourceConversions"],
        serde_json::json!([index])
    );
    assert_eq!(
        std::fs::read_to_string(profile(plan).join("contexthub.db")).unwrap(),
        "database-bytes-unchanged"
    );
    assert_eq!(
        snapshot(&open(&sandbox, true, MAXIMUM_ALLOWED).unwrap()).unwrap(),
        sandbox_before
    );
}

#[test]
fn inherited_source_exception_requires_a_repaired_direct_parent_and_private_directory() {
    let fixture = Fixture::new();
    let mut targets = collect(&fixture.request.plan).unwrap();
    let index = targets
        .iter()
        .position(|target| target.path == root(&fixture.request.plan))
        .unwrap();
    let before = "O:BAG:SYD:(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)";
    let after = before.replace("OICIID", "OICI");
    targets[index].before = before.into();
    assert!(legacy_conversion_after_parent(&targets, index, &after));
    targets[index].runtime = true;
    assert!(!legacy_conversion_after_parent(&targets, index, &after));
    targets[index].runtime = false;
    targets[index].directory = false;
    assert!(!legacy_conversion_after_parent(&targets, index, &after));
    targets[index].directory = true;
    targets[index].path = fixture.base.join("sibling");
    assert!(!legacy_conversion_after_parent(&targets, index, &after));
    targets[0].before = before.into();
    assert!(!legacy_conversion_after_parent(&targets, 0, &after));
}

#[test]
fn repair_journal_success_is_not_an_error_and_replaced_or_linked_journals_are_rejected() {
    let fixture = Fixture::new();
    apply(&fixture.request).unwrap();
    assert!(journal::recent(&fixture.request.plan).unwrap().is_empty());
    let journal_path = journal::path(&fixture.request).unwrap();
    let original = journal_path.with_extension("saved");
    std::fs::rename(&journal_path, &original).unwrap();
    drop(crate::security::create_new_restricted_file(&journal_path).unwrap());
    assert_eq!(
        journal::open_for_write(&fixture.request)
            .unwrap_err()
            .raw_os_error(),
        Some(Conflict::BackupChanged as i32)
    );
    std::fs::remove_file(&journal_path).unwrap();
    std::fs::hard_link(&original, &journal_path).unwrap();
    assert_eq!(
        journal::recent(&fixture.request.plan)
            .unwrap_err()
            .raw_os_error(),
        Some(UNSAFE_OBJECT)
    );
}

#[test]
fn rejects_unmanaged_profile_expired_plan_changed_anchor_and_bad_payload() {
    let fixture = Fixture::new();
    let mut plan = fixture.request.plan.clone();
    for id in ["../../Windows", "legacy", "signed-out"] {
        plan.profile_id = id.into();
        assert!(validate(&plan, &fixture.base).is_err());
    }
    for environment in ["dev", "prod"] {
        plan = fixture.request.plan.clone();
        plan.environment = environment.into();
        assert!(validate(&plan, &fixture.base).is_err());
    }
    plan = fixture.request.plan.clone();
    plan.expires_at = 0;
    assert!(validate(&plan, &fixture.base).is_err());
    assert!(validate(&fixture.request.plan, Path::new("C:\\other-user")).is_err());
    for text in ["invalid", "00", "a", "{}"] {
        assert!(decode(text).is_err());
    }
    assert_eq!(
        owner("O:BAG:SYD:P(A;;FA;;;SY)").unwrap(),
        ADMINISTRATORS_SID
    );
    assert_eq!(
        owner("O:S-1-5-21-1-2-3-4D:P(A;;FA;;;OW)").unwrap(),
        "S-1-5-21-1-2-3-4"
    );
    assert!(helper(&[HELPER_ARGUMENT.into(), encode(&fixture.request).unwrap()]).is_err());
}

#[test]
fn conflict_stages_survive_helper_exit_codes_and_do_not_modify_data() {
    let mut fixture = Fixture::new();
    let original =
        snapshot(&open(&root(&fixture.request.plan), true, MAXIMUM_ALLOWED).unwrap()).unwrap();
    let mut expired = fixture.request.plan.clone();
    expired.expires_at = 0;
    assert_eq!(
        validate(&expired, &fixture.base)
            .unwrap_err()
            .raw_os_error(),
        Some(Conflict::Expired as i32)
    );
    let other = fixture.base.join("another-profile");
    std::fs::create_dir(&other).unwrap();
    assert_eq!(
        validate(&fixture.request.plan, &other)
            .unwrap_err()
            .raw_os_error(),
        Some(Conflict::LocalDataChanged as i32)
    );

    let original_anchors = fixture.request.plan.anchors.clone();
    fixture.request.plan.anchors.clear();
    assert_eq!(
        apply(&fixture.request).unwrap_err().raw_os_error(),
        Some(Conflict::DirectoryChanged as i32)
    );
    fixture.request.plan.anchors = original_anchors;
    fixture.request.backup_identity = "replaced-backup".into();
    assert_eq!(
        apply(&fixture.request).unwrap_err().raw_os_error(),
        Some(Conflict::BackupChanged as i32)
    );
    assert_eq!(
        snapshot(&open(&root(&fixture.request.plan), true, MAXIMUM_ALLOWED).unwrap()).unwrap(),
        original
    );
    for code in 10011..=10018 {
        let roundtrip = io::Error::from_raw_os_error(code);
        assert!(super::super::conflict_reason(roundtrip.raw_os_error().unwrap()).is_some());
    }
}

#[test]
fn known_folder_spelling_changes_do_not_change_the_directory_identity() {
    let fixture = Fixture::new();
    // canonicalize returns a verbatim path; Known Folder APIs can return an
    // ordinary drive path and preserve a different casing for the same object.
    let ordinary = fixture
        .base
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .unwrap()
        .to_owned();
    let alias = PathBuf::from(ordinary.to_ascii_uppercase());
    assert_ne!(fixture.base, alias);
    assert_eq!(
        identity(&open(&fixture.base, true, 0).unwrap()).unwrap(),
        identity(&open(&alias, true, 0).unwrap()).unwrap()
    );
    validate(&fixture.request.plan, &alias).unwrap();
}

#[test]
fn owner_checks_normalize_sddl_aliases_instead_of_treating_them_as_other_accounts() {
    assert_eq!(
        owner("O:BAG:SYD:P(A;;FA;;;SY)").unwrap(),
        owner("O:S-1-5-32-544D:P(A;;FA;;;SY)").unwrap()
    );
    assert_eq!(owner("O:SYD:P(A;;FA;;;SY)").unwrap(), "S-1-5-18");
    assert!(owner("D:P(A;;FA;;;SY)").is_err());
    assert!(owner("invalid").is_err());
}

#[test]
fn newly_created_private_directories_and_lock_files_are_user_owned() {
    let fixture = Fixture::new();
    let directory = fixture.base.join("new-private/nested");
    crate::security::ensure_restricted_directory(&directory).unwrap();
    crate::security::verify_current_user_private_file(&directory).unwrap();
    let lock = directory.join("new.lock");
    drop(crate::security::open_restricted_file(&lock).unwrap());
    crate::security::verify_current_user_private_file(&lock).unwrap();
}

#[test]
fn startup_parent_accepts_the_live_windows_identity_and_rejects_forged_identity() {
    let pid = std::process::id();
    let fingerprint = crate::instance::process_start_fingerprint(pid)
        .unwrap()
        .unwrap();
    let token = current_token().unwrap();
    let sid = user_sid(&token).unwrap();
    native::verified_startup_parent(pid, &fingerprint, &sid).unwrap();
    assert!(native::verified_startup_parent(pid, "stale-process", &sid).is_err());
    assert!(native::verified_startup_parent(pid, &fingerprint, "S-1-0-0").is_err());
    native::verified_parent(pid, &fingerprint, &sid).unwrap();
    assert!(native::verified_parent(pid, "stale-process", &sid).is_err());
    assert!(native::verified_parent(pid, &fingerprint, "S-1-0-0").is_err());
}

#[test]
fn startup_diagnostics_reports_unreadable_metadata_without_reading_contents_or_changing_acl() {
    let fixture = Fixture::new();
    let plan = &fixture.request.plan;
    let path = profile(plan).join("run/instance.json");
    let handle = open(&path, false, MAXIMUM_ALLOWED).unwrap();
    set_security(
        &handle,
        &format!("O:{}D:P(A;;WDWO;;;OW)(A;;FA;;;SY)", plan.user_sid),
    )
    .unwrap();
    // The retained handle lets us prove the read-only diagnostic did not repair
    // or weaken an ACL it could not open with READ_CONTROL.
    let before = snapshot(&handle).unwrap();
    let result = diagnose_plan(
        plan,
        Diagnostics {
            process_elevated: false,
            errors: Vec::new(),
            repair_attempts: Vec::new(),
        },
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.scope == "instance" && error.os_error == Some(5))
    );
    assert_eq!(snapshot(&handle).unwrap(), before);
    let serialized = serde_json::to_string(&result).unwrap();
    for private in [
        plan.user_sid.as_str(),
        plan.local_data.as_str(),
        "original-instance",
        "database-bytes-unchanged",
        before.as_str(),
    ] {
        assert!(!serialized.contains(private));
    }
    set_security(&handle, &format!("O:{}{PRIVATE_DACL}", plan.user_sid)).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original-instance");
}

// Run explicitly in a dedicated elevated test process. Ordinary-user fixtures
// do not cover legacy objects whose default token owner was Administrators.
#[test]
#[ignore = "requires a dedicated elevated Windows test process"]
fn repairs_administrators_owned_legacy_tree_with_restore_privilege() {
    assert!(elevated(&current_token().unwrap()).unwrap());
    set_privilege("SeBackupPrivilege", true).unwrap();
    set_privilege("SeRestorePrivilege", true).unwrap();
    let mut fixture = Fixture::new();
    let plan = &mut fixture.request.plan;
    let sid = plan.user_sid.clone();
    let app = root(plan).parent().unwrap().to_owned();
    let inherited = format!("D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIID;FA;;;{sid})");
    let legacy = inherited.replace("D:AI", "D:");
    let private = "D:PAI(A;;FA;;;SY)(A;;FA;;;OW)";
    let runtime = "D:AI(A;;FA;;;BA)(A;;FA;;;SY)(A;;0x1200a9;;;S-1-5-5-0-12345)";
    let parent = open(&fixture.base, true, MAXIMUM_ALLOWED).unwrap();
    set_security(
        &parent,
        &format!("O:{sid}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{sid})"),
    )
    .unwrap();

    let existing = locations(plan)
        .into_iter()
        .filter(|(path, _, _)| path.exists())
        .collect::<Vec<_>>();
    for (path, directory, is_runtime) in &existing {
        let dacl = if path == &app {
            inherited.as_str()
        } else if path == &root(plan) || path == &root(plan).join("profiles") {
            legacy.as_str()
        } else if *is_runtime || path.to_string_lossy().contains("ai-runtime") {
            runtime
        } else {
            private
        };
        let sddl = format!("O:BA{dacl}");
        let handle = open(path, *directory, MAXIMUM_ALLOWED).unwrap();
        if dacl == legacy {
            // SetFileSecurityW is used only to recreate an old descriptor that
            // has inherited ACEs without SE_DACL_AUTO_INHERITED. Production
            // repairs use SetSecurityInfo on pinned handles exclusively.
            let text = wide(OsStr::new(&sddl));
            let mut descriptor = ptr::null_mut();
            // SAFETY: valid NUL-terminated fixture descriptor, retained below.
            check(unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    text.as_ptr(),
                    1,
                    &raw mut descriptor,
                    ptr::null_mut(),
                )
            })
            .unwrap();
            let _memory = LocalMemory(descriptor);
            let name = wide(path.as_os_str());
            // SAFETY: newly created isolated fixture only; no application data.
            check(unsafe {
                windows_sys::Win32::Security::SetFileSecurityW(
                    name.as_ptr(),
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    descriptor,
                )
            })
            .unwrap();
        } else {
            set_security(&handle, &sddl).unwrap();
        }
        assert_eq!(
            owner(&snapshot(&handle).unwrap()).unwrap(),
            ADMINISTRATORS_SID
        );
    }
    plan.anchors = anchors(plan).unwrap();
    let targets = collect(plan).unwrap();
    for target in &targets {
        assert_eq!(owner(&target.before).unwrap(), ADMINISTRATORS_SID);
    }
    drop(targets);
    apply(&fixture.request).unwrap();
    verify(&fixture.request.plan).unwrap();
    assert_eq!(
        std::fs::read_to_string(profile(&fixture.request.plan).join("contexthub.db")).unwrap(),
        "database-bytes-unchanged"
    );
}
