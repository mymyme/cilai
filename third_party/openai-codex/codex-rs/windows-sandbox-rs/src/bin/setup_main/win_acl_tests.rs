use super::DELETE;
use super::FILE_GENERIC_EXECUTE;
use super::FILE_GENERIC_READ;
use super::FILE_GENERIC_WRITE;
use super::GRANT_ACCESS;
use super::SetupMode;
use super::lock_sandbox_dir;
use super::resolve_sid;

#[test]
fn provision_only_locks_plain_directory_via_handle() {
    assert_originating_user_access_after_lock(SetupMode::ProvisionOnly);
}

#[test]
fn full_setup_keeps_originating_user_access() {
    assert_originating_user_access_after_lock(SetupMode::Full);
}

fn assert_originating_user_access_after_lock(mode: SetupMode) {
    let temp = tempfile::tempdir().expect("tempdir");
    let sandbox_bin = temp.path().join(".sandbox-bin");
    let real_user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").expect("current user domain"),
        std::env::var("USERNAME").expect("current user name")
    );
    let real_sid = resolve_sid(&real_user).expect("resolve real user SID");
    let real_user_sid =
        codex_windows_sandbox::string_from_sid_bytes(&real_sid).expect("format current user SID");
    pretty_assertions::assert_eq!(
        resolve_sid(&real_user_sid).expect("resolve exact user SID"),
        real_sid
    );

    lock_sandbox_dir(
        &sandbox_bin,
        &real_user_sid,
        &real_sid,
        GRANT_ACCESS,
        FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE,
        mode,
    )
    .expect("lock sandbox directory with the originating user SID");

    assert!(sandbox_bin.is_dir());
    std::fs::write(sandbox_bin.join("owner-access.txt"), "owner retains access")
        .expect("the originating user can still write after the directory is locked");
}
