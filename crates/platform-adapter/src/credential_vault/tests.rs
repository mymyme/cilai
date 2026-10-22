use super::*;

const SERVICE: &str = "cn.cilai.personal.workbench.vault.model";
const SECRET: &[u8] = b"synthetic-vault-secret-never-plaintext";

struct Fixture {
    base: PathBuf,
    vault: LocalCredentialVault,
}
impl Fixture {
    fn new() -> Self {
        let mut id = [0; 16];
        random(&mut id).unwrap();
        let id = format!("{:x}", Sha256::digest(id));
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("cilai-local-vault-{id}"));
        security::ensure_restricted_directory(&base).unwrap();
        let vault = LocalCredentialVault::new(base.join("credentials")).unwrap();
        Self { base, vault }
    }
    fn entry(&self, account: &str) -> PathBuf {
        self.vault
            .entry(&associated_data(SERVICE, account).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}

#[test]
fn vault_requires_an_explicit_ready_profile_and_never_creates_an_alternate_root() {
    let f = Fixture::new();
    assert!(LocalCredentialVault::new(PathBuf::from("credentials")).is_err());
    assert!(LocalCredentialVault::new(f.base.join("other")).is_err());
    let missing = f.base.join("missing-profile");
    assert!(LocalCredentialVault::new(missing.join("credentials")).is_err());
    assert!(!missing.exists());
    assert!(!f.vault.root.exists());
    let file = security::create_new_restricted_file(&f.vault.root).unwrap();
    drop(file);
    assert!(LocalCredentialVault::new(f.vault.root.clone()).is_err());
    assert!(f.vault.read(SERVICE, "one").is_err());
    assert!(f.vault.delete(SERVICE, "one").is_err());
}

#[test]
fn encrypted_roundtrip_atomic_replace_and_delete_do_not_expose_plaintext() {
    let f = Fixture::new();
    assert_eq!(f.vault.read(SERVICE, "one").unwrap(), None);
    assert!(!f.vault.root.exists());
    f.vault.write(SERVICE, "one", SECRET).unwrap();
    let first = fs::read(f.entry("one")).unwrap();
    assert!(!first.windows(SECRET.len()).any(|bytes| bytes == SECRET));
    let original_key = fs::read(f.vault.root.join(KEY_FILE)).unwrap();
    assert_eq!(original_key.len(), 32);
    f.vault.write(SERVICE, "one", SECRET).unwrap();
    assert_ne!(fs::read(f.entry("one")).unwrap(), first);
    assert_eq!(fs::read(f.vault.root.join(KEY_FILE)).unwrap(), original_key);
    let reopened = LocalCredentialVault::new(f.vault.root.clone()).unwrap();
    assert_eq!(
        reopened.read(SERVICE, "one").unwrap(),
        Some(SECRET.to_vec())
    );
    reopened.delete(SERVICE, "one").unwrap();
    reopened.delete(SERVICE, "one").unwrap();
    assert_eq!(reopened.read(SERVICE, "one").unwrap(), None);
    assert_eq!(fs::read(f.vault.root.join(KEY_FILE)).unwrap(), original_key);
    security::verify_restricted_path(&f.vault.root, true).unwrap();
    security::verify_current_user_private_file(&f.vault.root.join(KEY_FILE)).unwrap();
}

#[test]
fn authentication_binds_service_and_account_and_rejects_swapped_ciphertext() {
    let f = Fixture::new();
    f.vault.write(SERVICE, "one", SECRET).unwrap();
    f.vault.write(SERVICE, "two", b"second").unwrap();
    let one = fs::read(f.entry("one")).unwrap();
    let two = fs::read(f.entry("two")).unwrap();
    fs::write(f.entry("two"), &one).unwrap();
    assert!(f.vault.read(SERVICE, "two").is_err());
    assert!(f.vault.write(SERVICE, "two", b"replacement").is_err());
    assert!(f.vault.delete(SERVICE, "two").is_err());
    assert_eq!(fs::read(f.entry("two")).unwrap(), one);
    let aad = associated_data("cn.cilai.personal.workbench.vault.other", "one").unwrap();
    fs::copy(f.entry("one"), f.vault.entry(&aad)).unwrap();
    assert!(
        f.vault
            .read("cn.cilai.personal.workbench.vault.other", "one")
            .is_err()
    );
    assert_ne!(one, two);
}

#[test]
fn missing_or_invalid_key_never_replaces_existing_ciphertext_or_creates_a_key() {
    let f = Fixture::new();
    f.vault.write(SERVICE, "one", SECRET).unwrap();
    let key = f.vault.root.join(KEY_FILE);
    let saved = fs::read(&key).unwrap();
    let ciphertext = fs::read(f.entry("one")).unwrap();
    fs::remove_file(&key).unwrap();
    assert!(f.vault.read(SERVICE, "one").is_err());
    assert!(f.vault.write(SERVICE, "one", b"new").is_err());
    assert!(f.vault.delete(SERVICE, "one").is_err());
    assert!(!key.exists());
    assert_eq!(fs::read(f.entry("one")).unwrap(), ciphertext);
    let mut file = security::create_new_restricted_file(&key).unwrap();
    file.write_all(&saved[..31]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(f.vault.read(SERVICE, "one").is_err());
    assert!(f.vault.write(SERVICE, "one", b"new").is_err());
    fs::write(&key, &saved).unwrap();
    assert_eq!(f.vault.read(SERVICE, "one").unwrap(), Some(SECRET.to_vec()));
    let mut changed = ciphertext.clone();
    let last = changed.last_mut().unwrap();
    *last ^= 1;
    fs::write(f.entry("one"), &changed).unwrap();
    assert!(f.vault.read(SERVICE, "one").is_err());
    assert!(f.vault.write(SERVICE, "one", b"new").is_err());
    assert!(f.vault.delete(SERVICE, "one").is_err());
    assert_eq!(fs::read(f.entry("one")).unwrap(), changed);
}

#[test]
fn legacy_namespaces_unknown_layout_and_pending_writes_fail_closed() {
    let f = Fixture::new();
    assert!(
        f.vault
            .write("cn.cilai.personal.workbench.model", "one", SECRET)
            .is_err()
    );
    assert!(!f.vault.root.exists());
    f.vault.write(SERVICE, "one", SECRET).unwrap();
    let pending = f.vault.root.join(".pending-unconfirmed");
    let mut file = security::create_new_restricted_file(&pending).unwrap();
    file.write_all(b"incomplete").unwrap();
    drop(file);
    assert!(f.vault.read(SERVICE, "one").is_err());
    assert!(f.vault.delete(SERVICE, "one").is_err());
    assert!(f.vault.write(SERVICE, "two", SECRET).is_err());
    assert!(pending.exists());
    assert!(f.entry("one").exists());
    assert!(!f.entry("two").exists());
}

#[test]
fn concurrent_owner_lock_returns_busy_and_never_overwrites_other_work() {
    let f = Fixture::new();
    f.vault.write(SERVICE, "one", SECRET).unwrap();
    let lock = security::try_lock_restricted_file(&f.vault.root.join(LOCK_FILE))
        .unwrap()
        .unwrap();
    let vault = f.vault.clone();
    let error = std::thread::spawn(move || vault.write(SERVICE, "one", b"new").unwrap_err())
        .join()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    drop(lock);
    assert_eq!(f.vault.read(SERVICE, "one").unwrap(), Some(SECRET.to_vec()));
}

#[test]
fn hardlinks_are_rejected_for_key_ciphertext_and_lock_before_access() {
    for target in [KEY_FILE, LOCK_FILE, "entry"] {
        let f = Fixture::new();
        f.vault.write(SERVICE, "one", SECRET).unwrap();
        let path = if target == "entry" {
            f.entry("one")
        } else {
            f.vault.root.join(target)
        };
        fs::hard_link(&path, f.base.join("outside-hardlink")).unwrap();
        assert!(f.vault.read(SERVICE, "one").is_err());
        assert!(f.vault.write(SERVICE, "two", SECRET).is_err());
        assert!(f.vault.delete(SERVICE, "one").is_err());
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn symlink_ancestors_leafs_and_excess_permissions_are_rejected() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let f = Fixture::new();
    f.vault.write(SERVICE, "one", SECRET).unwrap();
    let alias = f.base.join("alias");
    symlink(&f.vault.root, &alias).unwrap();
    assert!(LocalCredentialVault::new(alias.join("credentials")).is_err());
    let key = f.vault.root.join(KEY_FILE);
    let outside = f.base.join("outside-key");
    fs::rename(&key, &outside).unwrap();
    symlink(&outside, &key).unwrap();
    assert!(f.vault.read(SERVICE, "one").is_err());
    fs::remove_file(&key).unwrap();
    fs::rename(&outside, &key).unwrap();
    fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(f.vault.read(SERVICE, "one").is_err());
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn dangling_root_never_means_missing_credentials_or_successful_cleanup() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    symlink(f.base.join("missing"), &f.vault.root).unwrap();
    assert!(LocalCredentialVault::new(f.vault.root.clone()).is_err());
    assert!(f.vault.read(SERVICE, "one").is_err());
    assert!(f.vault.write(SERVICE, "one", SECRET).is_err());
    assert!(f.vault.delete(SERVICE, "one").is_err());
    assert!(fs::symlink_metadata(&f.vault.root).unwrap().is_symlink());
    assert!(!f.base.join("missing").exists());
}
