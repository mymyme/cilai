//! 工作台资料专用的本地加密凭据；不访问系统凭据库。
use crate::{filesystem, security};
use ring::{
    aead,
    rand::{SecureRandom as _, SystemRandom},
};
use sha2::{Digest as _, Sha256};
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

const MAGIC: &[u8] = b"CILAI_VAULT_1\0";
const MAX_SECRET: usize = 256 * 1024;
const KEY_FILE: &str = "master.key";
const LOCK_FILE: &str = "vault.lock";

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "credential_vault_path_denied",
    )
}
fn corrupt() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "credential_vault_corrupt")
}

/// 显式绑定一个已就绪资料的 credentials 目录，无全局路径或系统存储回退。
#[derive(Clone)]
pub struct LocalCredentialVault {
    root: PathBuf,
}

impl LocalCredentialVault {
    /// # Errors
    /// 根目录非规范绝对路径、父资料未就绪或路径/权限不安全时失败。
    pub fn new(root: PathBuf) -> io::Result<Self> {
        if !root.is_absolute()
            || root.file_name().is_none_or(|name| name != "credentials")
            || root
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        {
            return Err(denied());
        }
        let parent = root.parent().ok_or_else(denied)?;
        validate_directories(parent)?;
        security::verify_restricted_path(parent, true)?;
        if filesystem::path_for_comparison(&fs::canonicalize(parent)?)
            != filesystem::path_for_comparison(parent)
        {
            return Err(denied());
        }
        let vault = Self { root };
        if entry_exists(&vault.root)? {
            vault.verify_root()?;
        }
        Ok(vault)
    }

    /// # Errors
    /// 文件损坏、密钥缺失、权限/路径异常或锁被占用时失败。
    pub fn read(&self, service: &str, account: &str) -> io::Result<Option<Vec<u8>>> {
        let aad = associated_data(service, account)?;
        let Some(_guard) = self.lock(false)? else {
            return Ok(None);
        };
        let entries = self.layout()?;
        let key = self.load_key(false, &entries)?;
        let path = self.entry(&aad);
        if !entry_exists(&path)? {
            return Ok(None);
        }
        let key = key.ok_or_else(corrupt)?;
        decrypt(
            &key,
            &aad,
            &read_file(&path, MAX_SECRET + MAGIC.len() + 32)?,
        )
        .map(Some)
    }

    /// # Errors
    /// 不覆盖缺钥或损坏的既有存储；随机源、原子写入、落盘或权限失败时返回错误。
    pub fn write(&self, service: &str, account: &str, value: &[u8]) -> io::Result<()> {
        let aad = associated_data(service, account)?;
        if value.is_empty() || value.len() > MAX_SECRET {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "credential_vault_value_invalid",
            ));
        }
        let _guard = self.lock(true)?.ok_or_else(corrupt)?;
        let entries = self.layout()?;
        let key = self.load_key(true, &entries)?.ok_or_else(corrupt)?;
        let path = self.entry(&aad);
        if entry_exists(&path)? {
            let mut previous = decrypt(
                &key,
                &aad,
                &read_file(&path, MAX_SECRET + MAGIC.len() + 32)?,
            )?;
            previous.fill(0);
        }
        let mut nonce = [0; 12];
        random(&mut nonce)?;
        let mut ciphertext = value.to_vec();
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(aad.as_slice()),
            &mut ciphertext,
        )
        .map_err(|_| corrupt())?;
        let mut bytes = Vec::with_capacity(MAGIC.len() + 12 + ciphertext.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&ciphertext);
        self.atomic_write(&path, &bytes)
    }

    /// # Errors
    /// 删除前必须确认当前密文有效；未知或损坏状态保留，不报告清理成功。
    pub fn delete(&self, service: &str, account: &str) -> io::Result<()> {
        let aad = associated_data(service, account)?;
        let Some(_guard) = self.lock(false)? else {
            return Ok(());
        };
        let entries = self.layout()?;
        let key = self.load_key(false, &entries)?;
        let path = self.entry(&aad);
        if !entry_exists(&path)? {
            return Ok(());
        }
        let mut previous = decrypt(
            &key.ok_or_else(corrupt)?,
            &aad,
            &read_file(&path, MAX_SECRET + MAGIC.len() + 32)?,
        )?;
        previous.fill(0);
        fs::remove_file(path)?;
        filesystem::sync_directory(&self.root)
    }

    fn verify_root(&self) -> io::Result<()> {
        validate_directories(&self.root)?;
        security::verify_restricted_path(&self.root, true)
    }

    fn lock(&self, create: bool) -> io::Result<Option<security::RestrictedFileLock>> {
        validate_directories(self.root.parent().ok_or_else(denied)?)?;
        if !entry_exists(&self.root)? {
            if !create {
                return Ok(None);
            }
            security::ensure_restricted_directory(&self.root)?;
            filesystem::sync_directory(self.root.parent().ok_or_else(denied)?)?;
        }
        self.verify_root()?;
        let path = self.root.join(LOCK_FILE);
        if entry_exists(&path)? {
            verify_file(&path)?;
        }
        let lock = security::try_lock_restricted_file(&path)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "credential_vault_busy"))?;
        verify_file(&path)?;
        Ok(Some(lock))
    }

    fn layout(&self) -> io::Result<Vec<PathBuf>> {
        self.verify_root()?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            verify_file(&path)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(corrupt)?;
            if matches!(name, KEY_FILE | LOCK_FILE) {
                continue;
            }
            if !name.strip_suffix(".vault").is_some_and(|id| {
                id.len() == 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) {
                return Err(corrupt());
            }
            entries.push(path);
        }
        Ok(entries)
    }

    fn entry(&self, aad: &[u8]) -> PathBuf {
        self.root.join(format!("{:x}.vault", Sha256::digest(aad)))
    }

    fn load_key(&self, create: bool, entries: &[PathBuf]) -> io::Result<Option<aead::LessSafeKey>> {
        let path = self.root.join(KEY_FILE);
        let mut bytes = if entry_exists(&path)? {
            read_file(&path, 32)?
        } else {
            if !entries.is_empty() {
                return Err(corrupt());
            }
            if !create {
                return Ok(None);
            }
            let mut bytes = vec![0; 32];
            random(&mut bytes)?;
            self.atomic_write(&path, &bytes)?;
            bytes
        };
        if bytes.len() != 32 {
            bytes.fill(0);
            return Err(corrupt());
        }
        let result = aead::UnboundKey::new(&aead::AES_256_GCM, &bytes)
            .map(aead::LessSafeKey::new)
            .map_err(|_| corrupt());
        bytes.fill(0);
        result.map(Some)
    }

    fn atomic_write(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        self.verify_root()?;
        if entry_exists(target)? {
            verify_file(target)?;
        }
        let mut suffix = [0; 16];
        random(&mut suffix)?;
        let suffix = format!("{:x}", Sha256::digest(suffix));
        let temporary = self.root.join(format!(".pending-{suffix}"));
        let mut file = security::create_new_restricted_file(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        verify_file(&temporary)?;
        if entry_exists(target)? {
            verify_file(target)?;
        }
        fs::rename(&temporary, target)?;
        filesystem::sync_directory(&self.root)?;
        verify_file(target)
    }
}

fn random(bytes: &mut [u8]) -> io::Result<()> {
    SystemRandom::new()
        .fill(bytes)
        .map_err(|_| io::Error::other("credential_vault_random_failed"))
}

fn entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn associated_data(service: &str, account: &str) -> io::Result<Vec<u8>> {
    if !service.starts_with("cn.cilai.personal.workbench.vault.")
        || service.len() > 256
        || account.is_empty()
        || account.len() > 1024
        || service.chars().chain(account.chars()).any(char::is_control)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "credential_vault_namespace_invalid",
        ));
    }
    let mut aad = b"cn.cilai.personal.workbench.vault/v1\0".to_vec();
    for value in [service, account] {
        aad.extend_from_slice(
            &u64::try_from(value.len())
                .map_err(|_| corrupt())?
                .to_be_bytes(),
        );
        aad.extend_from_slice(value.as_bytes());
    }
    Ok(aad)
}

fn validate_directories(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if !metadata.is_dir()
            || filesystem::is_link_or_reparse_point(&metadata)
            || filesystem::is_cloud_placeholder_file(&metadata)
        {
            return Err(denied());
        }
    }
    Ok(())
}

fn verify_file(path: &Path) -> io::Result<()> {
    let snapshot = filesystem::symlink_metadata_with_stable_identity(path)?;
    if !snapshot.metadata.is_file()
        || filesystem::is_link_or_reparse_point(&snapshot.metadata)
        || filesystem::is_cloud_placeholder_file(&snapshot.metadata)
    {
        return Err(denied());
    }
    security::verify_current_user_private_file(path)?;
    let file = filesystem::open_regular_file_no_follow(path, false)?;
    let opened = filesystem::opened_file_metadata_with_stable_identity(&file)?;
    if opened.stable_identity != snapshot.stable_identity || platform::link_count(&file)? != 1 {
        return Err(denied());
    }
    Ok(())
}

fn read_file(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    verify_file(path)?;
    let mut file = filesystem::open_regular_file_no_follow(path, false)?;
    let before = filesystem::opened_file_metadata_with_stable_identity(&file)?;
    if before.metadata.len() > u64::try_from(limit).map_err(|_| corrupt())?
        || platform::link_count(&file)? != 1
    {
        return Err(corrupt());
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(u64::try_from(limit + 1).map_err(|_| corrupt())?)
        .read_to_end(&mut bytes)?;
    let after = filesystem::symlink_metadata_with_stable_identity(path)?;
    if before.stable_identity != after.stable_identity
        || bytes.len() > limit
        || before.metadata.len() != after.metadata.len()
        || before.metadata.modified()? != after.metadata.modified()?
    {
        bytes.fill(0);
        return Err(corrupt());
    }
    Ok(bytes)
}

fn decrypt(key: &aead::LessSafeKey, aad: &[u8], bytes: &[u8]) -> io::Result<Vec<u8>> {
    if !bytes.starts_with(MAGIC) || bytes.len() < MAGIC.len() + 12 + 16 {
        return Err(corrupt());
    }
    let (nonce, ciphertext) = bytes[MAGIC.len()..].split_at(12);
    let nonce: [u8; 12] = nonce.try_into().map_err(|_| corrupt())?;
    let mut body = ciphertext.to_vec();
    let result = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(aad),
            &mut body,
        )
        .map(|plaintext| plaintext.to_vec())
        .map_err(|_| corrupt());
    body.fill(0);
    result
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod platform {
    use std::{fs::File, io, os::unix::fs::MetadataExt as _};
    pub(super) fn link_count(file: &File) -> io::Result<u64> {
        Ok(file.metadata()?.nlink())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use std::{fs::File, io, mem::MaybeUninit, os::windows::io::AsRawHandle as _};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    pub(super) fn link_count(file: &File) -> io::Result<u64> {
        let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // file持有本进程的有效文件句柄，输出缓冲区足够容纳API要求的结构。
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // 上述API成功后该结构已完整初始化。
        Ok(u64::from(unsafe { info.assume_init() }.nNumberOfLinks))
    }
}

#[cfg(test)]
mod tests;
