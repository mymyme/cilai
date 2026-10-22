use super::*;

#[test]
fn same_identity_helper_propagates_specific_failure_exit_code() {
    let child = spawn_same_identity_helper(
        "workspace_permissions::windows::tests::same_identity_helper_exit_probe",
        "--exact",
    )
    .unwrap();
    assert_eq!(
        wait_helper(child.as_raw_handle())
            .unwrap_err()
            .raw_os_error(),
        Some(10017)
    );
}

#[test]
fn same_identity_helper_exit_probe() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments
        == [
            "workspace_permissions::windows::tests::same_identity_helper_exit_probe",
            "--exact",
        ]
    {
        std::process::exit(10017);
    }
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cilai-permission-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn grants_only_directory_write_dac_preserving_owner_children_and_content() {
    let fixture = Fixture::new();
    let child = fixture.0.join("子目录");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(child.join("keep.txt"), "unchanged").unwrap();
    let target = open_directory(&fixture.0, MAXIMUM_ALLOWED).unwrap();
    let child_handle = open_directory(&child, READ_CONTROL).unwrap();
    let before = snapshot(&target).unwrap();
    let child_before = snapshot(&child_handle).unwrap();
    let test_sid = "S-1-5-21-312345678-312345679-312345680-23456";
    grant_directory_write_dac(&target, &before, test_sid).unwrap();
    let after = snapshot(&target).unwrap();
    assert!(after.contains(&format!("(A;;WD;;;{test_sid})")), "{after}");
    // The complete owner/group and DACL control prefix are preserved.
    assert_eq!(before.split('(').next(), after.split('(').next());
    for ace in before.split('(').skip(1) {
        assert!(after.contains(ace));
    }
    assert_eq!(snapshot(&child_handle).unwrap(), child_before);
    assert_eq!(
        std::fs::read_to_string(child.join("keep.txt")).unwrap(),
        "unchanged"
    );
    grant_directory_write_dac(&target, &after, test_sid).unwrap();
    assert_eq!(snapshot(&target).unwrap(), after);
}

#[test]
fn read_only_inspection_and_stale_confirmation_never_modify_acl() {
    let fixture = Fixture::new();
    let before = snapshot(&open_directory(&fixture.0, READ_CONTROL).unwrap()).unwrap();
    let checked = inspect(&fixture.0).unwrap();
    assert!(checked.can_update_permissions);
    assert_eq!(checked.security_backup, before);
    assert_eq!(
        decode(&payload(&checked.plan).unwrap()).unwrap().directory,
        checked.directory
    );
    let mut stale = checked.plan;
    stale.identity.push('x');
    assert_eq!(repair(&stale).unwrap_err().raw_os_error(), Some(RETRY));
    assert_eq!(
        snapshot(&open_directory(&fixture.0, READ_CONTROL).unwrap()).unwrap(),
        before
    );
}

#[test]
fn rejects_roots_networks_relative_paths_and_invalid_helper_payloads() {
    for path in [
        r"C:\",
        r"\\server\share\project",
        r"relative",
        r"C:\a\..\b",
        r"C:\a:b",
        r"C:\a.",
    ] {
        assert!(validate_path(Path::new(path)).is_err(), "{path}");
    }
    for input in ["not hex", "a", "00", "{}"] {
        assert!(decode(input).is_err());
    }
    assert!(validate_path(Path::new(r"C:\项目目录\中文 空格")).is_ok());
    assert!(helper(&[HELPER_ARGUMENT.into()]).is_err());
}

#[test]
fn new_private_files_have_exact_current_user_identity() {
    let fixture = Fixture::new();
    let file = fixture.0.join("instance.json");
    let _handle = crate::security::create_new_restricted_file(&file).unwrap();
    crate::security::verify_current_user_private_file(&file).unwrap();
    let before = std::fs::read(&file).unwrap();
    assert!(crate::security::create_new_restricted_file(&file).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn access_checks_remove_admin_membership_and_revert_even_on_failure() {
    use windows_sys::Win32::Security::CheckTokenMembership;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
    let before = user_sid(&current_token().unwrap()).unwrap();
    let error = with_current_user_access_checks(|| -> io::Result<()> {
        let mut raw = ptr::null_mut();
        // SAFETY: querying this thread's active impersonation token, owned below.
        check(unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &raw mut raw) })?;
        let thread_token = Handle(raw);
        assert_eq!(user_sid(&thread_token)?, before);
        let text = wide(OsStr::new("S-1-5-32-544"));
        let mut sid = ptr::null_mut();
        // SAFETY: fixed administrators SID and writable allocated output.
        check(unsafe { ConvertStringSidToSidW(text.as_ptr(), &raw mut sid) })?;
        let _sid = LocalMemory(sid);
        let mut member = 1;
        // SAFETY: NULL means the current thread token; SID remains allocated.
        check(unsafe { CheckTokenMembership(ptr::null_mut(), sid, &raw mut member) })?;
        assert_eq!(member, 0, "checks must not use administrative group grants");
        Err(changed())
    })
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(RETRY));
    let mut raw = ptr::null_mut();
    // SAFETY: querying only; absence proves the temporary impersonation was reverted.
    assert_eq!(
        unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &raw mut raw) },
        0
    );
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(1008));
    assert_eq!(user_sid(&current_token().unwrap()).unwrap(), before);
}
