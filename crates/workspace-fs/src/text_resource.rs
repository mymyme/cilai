use crate::{join_workspace_path, resolve_existing_workspace_path};
use contexthub_domain::WorkspaceRelativePath;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const DEFAULT_MAX_TEXT_RESOURCE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_TEXT_CHUNK_BYTES: usize = 64 * 1024;
pub const MIN_TEXT_CHUNK_BYTES: usize = 256;
const VALIDATION_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
}

impl TextEncoding {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf8Bom => "utf-8-bom",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedTextResourceRevision {
    pub size_bytes: Option<u64>,
    pub checksum_algorithm: Option<String>,
    pub checksum_value: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextResourceMetadata {
    pub encoding: TextEncoding,
    pub size_bytes: u64,
    pub content_bytes: u64,
    pub content_revision: String,
    pub first_offset: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextResourceChunk {
    pub text: String,
    pub offset: u64,
    pub next_offset: Option<u64>,
    pub content_revision: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextResourceErrorKind {
    RootUnavailable,
    TargetUnavailable,
    OutsideWorkspace,
    ReservedPath,
    NotRegularFile,
    FileTooLarge,
    BinaryContent,
    UnsupportedEncoding,
    SourceChanged,
    InvalidOffset,
    InvalidChunkLimit,
    SourceNotMaterialized,
    ReadFailed,
}

#[derive(Debug)]
pub struct TextResourceError {
    kind: TextResourceErrorKind,
    path: Option<PathBuf>,
    detected_encoding: Option<&'static str>,
    source: Option<io::Error>,
}

impl TextResourceError {
    #[must_use]
    pub const fn kind(&self) -> TextResourceErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn detected_encoding(&self) -> Option<&'static str> {
        self.detected_encoding
    }

    fn plain(kind: TextResourceErrorKind, path: Option<PathBuf>) -> Self {
        Self {
            kind,
            path,
            detected_encoding: None,
            source: None,
        }
    }

    fn encoding(path: PathBuf, detected_encoding: &'static str) -> Self {
        Self {
            kind: TextResourceErrorKind::UnsupportedEncoding,
            path: Some(path),
            detected_encoding: Some(detected_encoding),
            source: None,
        }
    }

    fn io(kind: TextResourceErrorKind, path: PathBuf, source: io::Error) -> Self {
        Self {
            kind,
            path: Some(path),
            detected_encoding: None,
            source: Some(source),
        }
    }
}

impl std::fmt::Display for TextResourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "text resource {:?}", self.kind)?;
        if let Some(path) = &self.path {
            write!(formatter, " at {}", path.display())?;
        }
        if let Some(encoding) = self.detected_encoding {
            write!(formatter, " ({encoding})")?;
        }
        if let Some(source) = &self.source {
            write!(formatter, ": {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for TextResourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileFingerprint {
    size_bytes: u64,
    modified_at: Option<SystemTime>,
    stable_identity: String,
}

#[derive(Debug)]
pub struct OpenedTextResource {
    root: PathBuf,
    relative_path: WorkspaceRelativePath,
    canonical_target: PathBuf,
    file: File,
    fingerprint: FileFingerprint,
    metadata: TextResourceMetadata,
}

impl OpenedTextResource {
    #[must_use]
    pub fn metadata(&self) -> &TextResourceMetadata {
        &self.metadata
    }

    /// 验证当前路径、打开句柄与签发时的资源身份仍一致。
    ///
    /// # Errors
    ///
    /// 文件被替换、删除、重定向或元数据变化时返回 `SourceChanged`。
    pub fn verify_unchanged(&self) -> Result<(), TextResourceError> {
        self.verify_current()
    }

    /// 从已签发内容 revision 的原始字节 offset 读取一个 UTF-8 安全分块。
    ///
    /// # Errors
    ///
    /// offset 或上限无效，路径、身份或元数据变化，或读取失败时返回错误。
    pub fn read_chunk(
        &mut self,
        offset: u64,
        max_bytes: usize,
    ) -> Result<TextResourceChunk, TextResourceError> {
        if !(MIN_TEXT_CHUNK_BYTES..=MAX_TEXT_CHUNK_BYTES).contains(&max_bytes) {
            return Err(TextResourceError::plain(
                TextResourceErrorKind::InvalidChunkLimit,
                None,
            ));
        }
        if offset < self.metadata.first_offset || offset > self.metadata.size_bytes {
            return Err(TextResourceError::plain(
                TextResourceErrorKind::InvalidOffset,
                None,
            ));
        }
        self.verify_current()?;
        if offset == self.metadata.size_bytes {
            return Ok(TextResourceChunk {
                text: String::new(),
                offset,
                next_offset: None,
                content_revision: self.metadata.content_revision.clone(),
            });
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|source| self.read_error(source))?;
        let remaining = self.metadata.size_bytes - offset;
        let requested = usize::try_from(remaining.min(max_bytes as u64)).map_err(|_| {
            TextResourceError::plain(TextResourceErrorKind::InvalidChunkLimit, None)
        })?;
        let mut bytes = vec![0_u8; requested];
        self.file
            .read_exact(&mut bytes)
            .map_err(|source| self.read_error(source))?;
        let consumed = utf8_prefix_len(&bytes)
            .ok_or_else(|| TextResourceError::plain(TextResourceErrorKind::InvalidOffset, None))?;
        if consumed == 0 {
            return Err(TextResourceError::plain(
                TextResourceErrorKind::InvalidOffset,
                None,
            ));
        }
        bytes.truncate(consumed);
        let text = String::from_utf8(bytes)
            .map_err(|_| TextResourceError::plain(TextResourceErrorKind::SourceChanged, None))?;
        let consumed_u64 = u64::try_from(consumed).map_err(|_| {
            TextResourceError::plain(TextResourceErrorKind::InvalidChunkLimit, None)
        })?;
        let next = offset
            .checked_add(consumed_u64)
            .ok_or_else(|| TextResourceError::plain(TextResourceErrorKind::InvalidOffset, None))?;
        self.verify_current()?;
        Ok(TextResourceChunk {
            text,
            offset,
            next_offset: (next < self.metadata.size_bytes).then_some(next),
            content_revision: self.metadata.content_revision.clone(),
        })
    }

    fn verify_current(&self) -> Result<(), TextResourceError> {
        let unresolved = join_workspace_path(&self.root, &self.relative_path);
        if fs::symlink_metadata(&unresolved)
            .map_or(true, |metadata| metadata.file_type().is_symlink())
        {
            return Err(TextResourceError::plain(
                TextResourceErrorKind::SourceChanged,
                Some(unresolved),
            ));
        }
        let resolved =
            resolve_existing_workspace_path(&self.root, &self.relative_path).map_err(|_| {
                TextResourceError::plain(
                    TextResourceErrorKind::SourceChanged,
                    Some(unresolved.clone()),
                )
            })?;
        if resolved != self.canonical_target {
            return Err(TextResourceError::plain(
                TextResourceErrorKind::SourceChanged,
                Some(resolved),
            ));
        }
        let path_snapshot =
            contexthub_platform_adapter::filesystem::metadata_with_stable_identity(&resolved)
                .map_err(|source| {
                    TextResourceError::io(
                        TextResourceErrorKind::SourceChanged,
                        resolved.clone(),
                        source,
                    )
                })?;
        let path_fingerprint = fingerprint(path_snapshot);
        let handle_snapshot =
            contexthub_platform_adapter::filesystem::opened_file_metadata_with_stable_identity(
                &self.file,
            )
            .map_err(|source| {
                TextResourceError::io(
                    TextResourceErrorKind::SourceChanged,
                    self.canonical_target.clone(),
                    source,
                )
            })?;
        let handle_fingerprint = fingerprint(handle_snapshot);
        if path_fingerprint != self.fingerprint || handle_fingerprint != self.fingerprint {
            return Err(TextResourceError::plain(
                TextResourceErrorKind::SourceChanged,
                Some(self.canonical_target.clone()),
            ));
        }
        Ok(())
    }

    fn read_error(&self, source: io::Error) -> TextResourceError {
        TextResourceError::io(
            TextResourceErrorKind::ReadFailed,
            self.canonical_target.clone(),
            source,
        )
    }
}

/// 安全打开、流式验证并绑定一个 UTF-8 文本资源 revision。
///
/// # Errors
///
/// 路径不安全、目标不是普通文件、超过上限、编码不支持、数据库观察已过期或读取期间变化时返回错误。
#[expect(
    clippy::too_many_lines,
    reason = "keeps path, type, size, revision, encoding, and post-read identity validation in one auditable open sequence"
)]
pub fn open_text_resource(
    root: &Path,
    relative_path: &WorkspaceRelativePath,
    expected: &ExpectedTextResourceRevision,
    max_size_bytes: u64,
) -> Result<OpenedTextResource, TextResourceError> {
    if relative_path
        .segments()
        .any(crate::is_workspace_metadata_directory)
    {
        return Err(TextResourceError::plain(
            TextResourceErrorKind::ReservedPath,
            None,
        ));
    }
    let unresolved = join_workspace_path(root, relative_path);
    let unresolved_metadata = fs::symlink_metadata(&unresolved).map_err(|source| {
        TextResourceError::io(
            TextResourceErrorKind::TargetUnavailable,
            unresolved.clone(),
            source,
        )
    })?;
    if unresolved_metadata.file_type().is_symlink() || !unresolved_metadata.is_file() {
        return Err(TextResourceError::plain(
            TextResourceErrorKind::NotRegularFile,
            Some(unresolved),
        ));
    }
    if contexthub_platform_adapter::filesystem::is_cloud_placeholder_file(&unresolved_metadata) {
        return Err(TextResourceError::plain(
            TextResourceErrorKind::SourceNotMaterialized,
            Some(unresolved),
        ));
    }
    let canonical_target =
        resolve_existing_workspace_path(root, relative_path).map_err(|error| {
            let kind = match error.kind() {
                crate::WorkspaceResolveErrorKind::RootUnavailable => {
                    TextResourceErrorKind::RootUnavailable
                }
                crate::WorkspaceResolveErrorKind::TargetUnavailable => {
                    TextResourceErrorKind::TargetUnavailable
                }
                crate::WorkspaceResolveErrorKind::OutsideWorkspace => {
                    TextResourceErrorKind::OutsideWorkspace
                }
            };
            TextResourceError::plain(kind, Some(unresolved.clone()))
        })?;
    let mut file =
        contexthub_platform_adapter::filesystem::open_workspace_file_for_read(&canonical_target)
            .map_err(|source| {
                TextResourceError::io(
                    TextResourceErrorKind::TargetUnavailable,
                    canonical_target.clone(),
                    source,
                )
            })?;
    let initial_snapshot =
        contexthub_platform_adapter::filesystem::opened_file_metadata_with_stable_identity(&file)
            .map_err(|source| {
            TextResourceError::io(
                TextResourceErrorKind::TargetUnavailable,
                canonical_target.clone(),
                source,
            )
        })?;
    if !initial_snapshot.metadata.is_file() {
        return Err(TextResourceError::plain(
            TextResourceErrorKind::NotRegularFile,
            Some(canonical_target),
        ));
    }
    let initial_fingerprint = fingerprint(initial_snapshot);
    if initial_fingerprint.size_bytes > max_size_bytes {
        return Err(TextResourceError::plain(
            TextResourceErrorKind::FileTooLarge,
            Some(canonical_target),
        ));
    }
    if expected
        .size_bytes
        .is_some_and(|size| size != initial_fingerprint.size_bytes)
    {
        return Err(TextResourceError::plain(
            TextResourceErrorKind::SourceChanged,
            Some(canonical_target),
        ));
    }
    let (encoding, first_offset, content_revision) =
        validate_utf8_and_checksum(&mut file, &canonical_target)?;
    let final_snapshot =
        contexthub_platform_adapter::filesystem::opened_file_metadata_with_stable_identity(&file)
            .map_err(|source| {
            TextResourceError::io(
                TextResourceErrorKind::ReadFailed,
                canonical_target.clone(),
                source,
            )
        })?;
    let final_fingerprint = fingerprint(final_snapshot);
    if final_fingerprint != initial_fingerprint
        || expected.checksum_algorithm.is_some() != expected.checksum_value.is_some()
        || expected
            .checksum_algorithm
            .as_deref()
            .is_some_and(|algorithm| algorithm != "sha256")
        || expected
            .checksum_value
            .as_deref()
            .is_some_and(|value| value != content_revision)
    {
        return Err(TextResourceError::plain(
            TextResourceErrorKind::SourceChanged,
            Some(canonical_target),
        ));
    }
    file.seek(SeekFrom::Start(first_offset)).map_err(|source| {
        TextResourceError::io(
            TextResourceErrorKind::ReadFailed,
            canonical_target.clone(),
            source,
        )
    })?;
    let canonical_root = fs::canonicalize(root).map_err(|source| {
        TextResourceError::io(
            TextResourceErrorKind::RootUnavailable,
            root.to_owned(),
            source,
        )
    })?;
    Ok(OpenedTextResource {
        root: canonical_root,
        relative_path: relative_path.clone(),
        canonical_target,
        file,
        fingerprint: initial_fingerprint.clone(),
        metadata: TextResourceMetadata {
            encoding,
            size_bytes: initial_fingerprint.size_bytes,
            content_bytes: initial_fingerprint.size_bytes - first_offset,
            content_revision,
            first_offset,
        },
    })
}

fn validate_utf8_and_checksum(
    file: &mut File,
    path: &Path,
) -> Result<(TextEncoding, u64, String), TextResourceError> {
    file.seek(SeekFrom::Start(0)).map_err(|source| {
        TextResourceError::io(TextResourceErrorKind::ReadFailed, path.to_owned(), source)
    })?;
    let mut prefix = [0_u8; 3];
    let prefix_len = file.read(&mut prefix).map_err(|source| {
        TextResourceError::io(TextResourceErrorKind::ReadFailed, path.to_owned(), source)
    })?;
    if prefix_len >= 2 && prefix[..2] == [0xff, 0xfe] {
        return Err(TextResourceError::encoding(path.to_owned(), "utf-16le"));
    }
    if prefix_len >= 2 && prefix[..2] == [0xfe, 0xff] {
        return Err(TextResourceError::encoding(path.to_owned(), "utf-16be"));
    }
    let (encoding, first_offset) = if prefix_len == 3 && prefix == [0xef, 0xbb, 0xbf] {
        (TextEncoding::Utf8Bom, 3)
    } else {
        (TextEncoding::Utf8, 0)
    };
    file.seek(SeekFrom::Start(0)).map_err(|source| {
        TextResourceError::io(TextResourceErrorKind::ReadFailed, path.to_owned(), source)
    })?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; VALIDATION_BUFFER_BYTES];
    let mut carry = Vec::with_capacity(4);
    loop {
        let read = file.read(&mut buffer).map_err(|source| {
            TextResourceError::io(TextResourceErrorKind::ReadFailed, path.to_owned(), source)
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        if buffer[..read].contains(&0) {
            return Err(TextResourceError::plain(
                TextResourceErrorKind::BinaryContent,
                Some(path.to_owned()),
            ));
        }
        carry.extend_from_slice(&buffer[..read]);
        match std::str::from_utf8(&carry) {
            Ok(_) => carry.clear(),
            Err(error) if error.error_len().is_none() => {
                let tail = carry.split_off(error.valid_up_to());
                carry = tail;
            }
            Err(_) => {
                return Err(TextResourceError::encoding(path.to_owned(), "unknown"));
            }
        }
    }
    if !carry.is_empty() {
        return Err(TextResourceError::encoding(path.to_owned(), "unknown"));
    }
    Ok((encoding, first_offset, format!("{:x}", digest.finalize())))
}

fn utf8_prefix_len(bytes: &[u8]) -> Option<usize> {
    match std::str::from_utf8(bytes) {
        Ok(_) => Some(bytes.len()),
        Err(error) if error.error_len().is_none() => Some(error.valid_up_to()),
        Err(_) => None,
    }
}

fn fingerprint(
    snapshot: contexthub_platform_adapter::filesystem::FileMetadataSnapshot,
) -> FileFingerprint {
    FileFingerprint {
        size_bytes: snapshot.metadata.len(),
        modified_at: snapshot.metadata.modified().ok(),
        stable_identity: snapshot.stable_identity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn metadata_from_current_and_legacy_names_is_not_read_as_business_text() {
        let fixture = Fixture::new("metadata");
        for reserved in [".cilai", ".CILAI", ".contexthub", "nested/.cilai"] {
            fs::create_dir_all(fixture.root.join(reserved)).unwrap();
            fs::write(fixture.root.join(reserved).join("workspace.json"), b"{}").unwrap();
            assert_eq!(
                open(
                    &fixture,
                    &format!("{reserved}/workspace.json"),
                    &expected(),
                    100
                )
                .unwrap_err()
                .kind(),
                TextResourceErrorKind::ReservedPath
            );
        }
    }

    #[test]
    fn utf8_bom_and_multibyte_boundaries_are_read_without_large_messages() {
        let fixture = Fixture::new("utf8");
        let text = format!("{}界end", "a".repeat(255));
        fs::write(
            fixture.root.join("note.md"),
            [b"\xef\xbb\xbf", text.as_bytes()].concat(),
        )
        .unwrap();
        let mut resource = open_text_resource(
            &fixture.root,
            &WorkspaceRelativePath::parse("note.md").unwrap(),
            &expected(),
            DEFAULT_MAX_TEXT_RESOURCE_BYTES,
        )
        .unwrap();
        assert_eq!(resource.metadata().encoding, TextEncoding::Utf8Bom);
        let first = resource.read_chunk(3, 256).unwrap();
        assert_eq!(first.text.len(), 255);
        let second = resource
            .read_chunk(first.next_offset.unwrap(), 256)
            .unwrap();
        assert_eq!(second.text, "界end");
        assert!(second.next_offset.is_none());
    }

    #[test]
    fn unsupported_binary_size_and_stale_checksum_are_distinct() {
        let fixture = Fixture::new("errors");
        fs::write(fixture.root.join("binary.txt"), b"a\0b").unwrap();
        assert_eq!(
            open(&fixture, "binary.txt", &expected(), 100)
                .unwrap_err()
                .kind(),
            TextResourceErrorKind::BinaryContent
        );
        fs::write(fixture.root.join("utf16.txt"), [0xff, 0xfe, b'a', 0]).unwrap();
        let utf16 = open(&fixture, "utf16.txt", &expected(), 100).unwrap_err();
        assert_eq!(utf16.kind(), TextResourceErrorKind::UnsupportedEncoding);
        assert_eq!(utf16.detected_encoding(), Some("utf-16le"));
        fs::write(fixture.root.join("large.txt"), b"large").unwrap();
        assert_eq!(
            open(&fixture, "large.txt", &expected(), 4)
                .unwrap_err()
                .kind(),
            TextResourceErrorKind::FileTooLarge
        );
        let stale = ExpectedTextResourceRevision {
            size_bytes: Some(5),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some("stale".to_owned()),
        };
        assert_eq!(
            open(&fixture, "large.txt", &stale, 100).unwrap_err().kind(),
            TextResourceErrorKind::SourceChanged
        );
    }

    #[test]
    fn replacement_after_open_invalidates_old_resource() {
        let fixture = Fixture::new("replace");
        fs::write(fixture.root.join("note.txt"), "old text").unwrap();
        let mut resource = open(&fixture, "note.txt", &expected(), 100).unwrap();
        let replacement = fixture.root.join("replacement");
        fs::write(&replacement, "new text").unwrap();
        fs::rename(replacement, fixture.root.join("note.txt")).unwrap();
        assert_eq!(
            resource.read_chunk(0, 256).unwrap_err().kind(),
            TextResourceErrorKind::SourceChanged
        );
    }

    fn expected() -> ExpectedTextResourceRevision {
        ExpectedTextResourceRevision {
            size_bytes: None,
            checksum_algorithm: None,
            checksum_value: None,
        }
    }

    fn open(
        fixture: &Fixture,
        path: &str,
        expected: &ExpectedTextResourceRevision,
        limit: u64,
    ) -> Result<OpenedTextResource, TextResourceError> {
        open_text_resource(
            &fixture.root,
            &WorkspaceRelativePath::parse(path).unwrap(),
            expected,
            limit,
        )
    }

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("ch-text-{label}-{}-{nanos}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self { root }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}
