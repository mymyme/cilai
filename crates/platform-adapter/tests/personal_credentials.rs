use contexthub_platform_adapter::{credential_vault::LocalCredentialVault, security};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const SERVICE: &str = "cn.cilai.personal.workbench.vault.model";
const VALUE: &[u8] = b"synthetic-personal-component-credential";
const CHILD_ROOT: &str = "CILAI_LOCAL_VAULT_PROBE_ROOT";

#[test]
fn local_credential_survives_process_restart_and_is_deleted() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let parent = std::env::temp_dir().join(format!(
        "cilai-local-vault-probe-{}-{nonce}",
        std::process::id()
    ));
    security::ensure_restricted_directory(&parent).unwrap();
    let root = fs::canonicalize(&parent).unwrap().join("credentials");
    let store = LocalCredentialVault::new(root.clone()).unwrap();
    assert!(
        store
            .read(SERVICE, "profile:connection:1")
            .unwrap()
            .is_none()
    );
    store.write(SERVICE, "profile:connection:1", VALUE).unwrap();
    assert!(store.read(SERVICE, "profile:other:1").unwrap().is_none());
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "read_own_local_credential_in_a_new_process",
        ])
        .env(CHILD_ROOT, &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for entry in fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        security::verify_current_user_private_file(&path).unwrap();
        assert!(
            !fs::read(path)
                .unwrap()
                .windows(VALUE.len())
                .any(|part| part == VALUE)
        );
    }
    store.delete(SERVICE, "profile:connection:1").unwrap();
    assert!(
        store
            .read(SERVICE, "profile:connection:1")
            .unwrap()
            .is_none()
    );
    fs::remove_dir_all(parent).unwrap();
}

#[test]
#[ignore = "仅由本地凭据跨进程测试在其私有目录中调用"]
fn read_own_local_credential_in_a_new_process() {
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).expect("private fixture root"));
    let store = LocalCredentialVault::new(root).unwrap();
    let mut value = store
        .read(SERVICE, "profile:connection:1")
        .unwrap()
        .unwrap();
    let matches = value == VALUE;
    value.fill(0);
    assert!(matches);
}
