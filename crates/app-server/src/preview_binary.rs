use base64::Engine as _;
use contexthub_application::PreviewFileRecord;
use contexthub_domain::WorkspaceRelativePath;
use contexthub_platform_adapter::filesystem as fs;
use contexthub_protocol::{ReadBinaryPreviewChunkResult, ReadBinaryPreviewResult};
use contexthub_workspace_fs::resolve_existing_workspace_path;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct VerifiedBinary {
    bytes: Vec<u8>,
    content_type: String,
    revision: String,
    root: PathBuf,
    relative: WorkspaceRelativePath,
    path: PathBuf,
    observed: fs::FileMetadataSnapshot,
}

pub(super) fn read(record: &PreviewFileRecord) -> Result<VerifiedBinary, &'static str> {
    if record.materialization_state != "materialized" {
        return Err("preview_file_not_materialized");
    }
    let relative = WorkspaceRelativePath::parse(&record.relative_path)
        .map_err(|_| "invalid_preview_request")?;
    if relative
        .segments()
        .any(contexthub_workspace_fs::is_workspace_metadata_directory)
    {
        return Err("preview_permission_denied");
    }
    let root = Path::new(&record.workspace_root);
    let path = resolve_existing_workspace_path(root, &relative)
        .map_err(|_| "preview_permission_denied")?;
    let observed =
        fs::symlink_metadata_with_stable_identity(&path).map_err(|_| "preview_file_missing")?;
    if !observed.metadata.is_file() || observed.metadata.file_type().is_symlink() {
        return Err("preview_file_not_regular");
    }
    if fs::is_cloud_placeholder_file(&observed.metadata) {
        return Err("preview_file_not_materialized");
    }
    if observed.metadata.len() > MAX_BYTES {
        return Err("preview_file_too_large");
    }
    let mut file =
        fs::open_stable_workspace_file_for_read(&path).map_err(|_| "preview_read_failed")?;
    let before =
        fs::opened_file_metadata_with_stable_identity(&file).map_err(|_| "preview_read_failed")?;
    if before.stable_identity != observed.stable_identity || before.metadata.len() > MAX_BYTES {
        return Err("preview_source_changed");
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "preview_read_failed")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("preview_file_too_large");
    }
    let after = fs::opened_file_metadata_with_stable_identity(&file)
        .map_err(|_| "preview_source_changed")?;
    let current_path =
        resolve_existing_workspace_path(root, &relative).map_err(|_| "preview_source_changed")?;
    let current = fs::symlink_metadata_with_stable_identity(&current_path)
        .map_err(|_| "preview_source_changed")?;
    if path != current_path
        || before.stable_identity != current.stable_identity
        || before.stable_identity != after.stable_identity
        || before.metadata.len() != after.metadata.len()
        || before.metadata.len() != current.metadata.len()
        || before.metadata.len() != bytes.len() as u64
        || before.metadata.modified().ok() != after.metadata.modified().ok()
        || before.metadata.modified().ok() != current.metadata.modified().ok()
    {
        return Err("preview_source_changed");
    }
    let revision = format!("{:x}", Sha256::digest(&bytes));
    if record
        .size_bytes
        .is_some_and(|expected| u64::try_from(expected).ok() != Some(bytes.len() as u64))
        || (record.checksum_algorithm.as_deref() == Some("sha256")
            && record
                .checksum_value
                .as_deref()
                .is_some_and(|value| value != revision))
    {
        return Err("preview_source_changed");
    }
    let extension = path
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let content_type = match extension.as_str() {
        "png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => "image/png",
        "jpg" | "jpeg" if bytes.starts_with(&[255, 216, 255]) => "image/jpeg",
        "gif" if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") => "image/gif",
        "webp" if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") => "image/webp",
        "bmp" if bytes.starts_with(b"BM") => "image/bmp",
        "docx" | "pptx" | "xlsx" | "xmind" if bytes.starts_with(b"PK\x03\x04") => "application/zip",
        _ => return Err("preview_binary_format_invalid"),
    };
    Ok(VerifiedBinary {
        bytes,
        revision,
        content_type: content_type.to_owned(),
        root: root.to_path_buf(),
        relative,
        path,
        observed: after,
    })
}

impl VerifiedBinary {
    fn verify_path(&self) -> Result<(), &'static str> {
        let path = resolve_existing_workspace_path(&self.root, &self.relative)
            .map_err(|_| "preview_source_changed")?;
        let current = fs::symlink_metadata_with_stable_identity(&path)
            .map_err(|_| "preview_source_changed")?;
        if path != self.path
            || !current.metadata.is_file()
            || fs::is_link_or_reparse_point(&current.metadata)
            || fs::is_cloud_placeholder_file(&current.metadata)
            || current.stable_identity != self.observed.stable_identity
            || current.metadata.len() != self.observed.metadata.len()
            || current.metadata.modified().ok() != self.observed.metadata.modified().ok()
        {
            return Err("preview_source_changed");
        }
        Ok(())
    }
}

const MAX_ACTIVE: usize = 8;
const MAX_TOTAL: u64 = 64 * 1024 * 1024;
const CHUNK_BYTES: usize = 128 * 1024;
const TICKET_LIFETIME_MS: i64 = 2 * 60 * 1000;

struct Entry {
    owner: String,
    expires_at: i64,
    data: VerifiedBinary,
}

#[derive(Default)]
pub(super) struct BinaryPreviewTickets(HashMap<String, Entry>);

impl BinaryPreviewTickets {
    pub(super) fn issue(
        &mut self,
        owner: &str,
        now: i64,
        record: &PreviewFileRecord,
    ) -> Result<ReadBinaryPreviewResult, &'static str> {
        self.0.retain(|_, entry| entry.expires_at > now);
        let occupied = self
            .0
            .values()
            .map(|entry| entry.data.bytes.len() as u64)
            .sum::<u64>();
        if self.0.len() >= MAX_ACTIVE || occupied >= MAX_TOTAL {
            return Err("preview_ticket_capacity_reached");
        }
        let data = read(record)?;
        let size = data.bytes.len() as u64;
        if occupied + size > MAX_TOTAL {
            return Err("preview_ticket_capacity_reached");
        }
        let ticket_id = format!("binary-preview-{}", uuid::Uuid::new_v4());
        let result = ReadBinaryPreviewResult {
            ticket_id: ticket_id.clone(),
            content_type: data.content_type.clone(),
            content_revision: data.revision.clone(),
            size_bytes: size,
        };
        self.0.insert(
            ticket_id,
            Entry {
                owner: owner.to_owned(),
                expires_at: now.saturating_add(TICKET_LIFETIME_MS),
                data,
            },
        );
        Ok(result)
    }

    pub(super) fn read(
        &mut self,
        owner: &str,
        ticket: &str,
        offset: u64,
        now: i64,
    ) -> Result<ReadBinaryPreviewChunkResult, &'static str> {
        let entry = self
            .0
            .get(ticket)
            .filter(|entry| entry.owner == owner)
            .ok_or("preview_ticket_not_found")?;
        if entry.expires_at <= now {
            self.0.remove(ticket);
            return Err("preview_ticket_expired");
        }
        if let Err(error) = entry.data.verify_path() {
            self.0.remove(ticket);
            return Err(error);
        }
        let start = usize::try_from(offset).map_err(|_| "invalid_preview_request")?;
        if start % CHUNK_BYTES != 0 || start >= entry.data.bytes.len() {
            return Err("invalid_preview_request");
        }
        let end = (start + CHUNK_BYTES).min(entry.data.bytes.len());
        Ok(ReadBinaryPreviewChunkResult {
            offset,
            content_base64: base64::engine::general_purpose::STANDARD
                .encode(&entry.data.bytes[start..end]),
        })
    }

    pub(super) fn release(&mut self, owner: &str, ticket: &str) -> bool {
        self.0.get(ticket).is_some_and(|entry| entry.owner == owner)
            && self.0.remove(ticket).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_domain::WorkspaceId;
    use std::fs;

    fn record(root: &Path, path: &str, size: i64) -> PreviewFileRecord {
        PreviewFileRecord {
            workspace_id: WorkspaceId::new("preview"),
            local_file_id: "image".to_owned(),
            workspace_root: root.to_string_lossy().into_owned(),
            relative_path: path.to_owned(),
            materialization_state: "materialized".to_owned(),
            file_type: "file".to_owned(),
            size_bytes: Some(size),
            modified_at_fs: None,
            checksum_algorithm: None,
            checksum_value: None,
            local_change_state: None,
            baseline_text_content: None,
            baseline_content_revision: None,
            baseline_confirmed_at: None,
        }
    }
    #[test]
    fn reads_binary_in_memory_and_rejects_changed_and_forged_images() {
        let root =
            std::env::temp_dir().join(format!("cilai-binary-preview-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let bytes = b"\x89PNG\r\n\x1a\ncontent";
        fs::write(root.join("a.png"), bytes).unwrap();
        let source = record(&root, "a.png", i64::try_from(bytes.len()).unwrap());
        let result = read(&source).unwrap();
        assert_eq!(result.content_type, "image/png");
        assert_eq!(result.bytes, bytes);
        let mut tickets = BinaryPreviewTickets::default();
        let opened = tickets.issue("owner", 1, &source).unwrap();
        assert!(!tickets.release("different-owner", &opened.ticket_id));
        assert_eq!(
            tickets
                .read("different-owner", &opened.ticket_id, 0, 2)
                .unwrap_err(),
            "preview_ticket_not_found"
        );
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(
                    tickets
                        .read("owner", &opened.ticket_id, 0, 2)
                        .unwrap()
                        .content_base64
                )
                .unwrap(),
            bytes
        );
        let expired = tickets.issue("owner", 1, &source).unwrap();
        assert_eq!(
            tickets
                .read("owner", &expired.ticket_id, 0, TICKET_LIFETIME_MS + 1)
                .unwrap_err(),
            "preview_ticket_expired"
        );
        let mut changed = source.clone();
        changed.checksum_algorithm = Some("sha256".to_owned());
        changed.checksum_value = Some("0".repeat(64));
        assert_eq!(read(&changed).unwrap_err(), "preview_source_changed");
        fs::write(root.join("a.png"), b"<script>").unwrap();
        assert_eq!(
            tickets.read("owner", &opened.ticket_id, 0, 3).unwrap_err(),
            "preview_source_changed"
        );
        assert_eq!(
            read(&record(&root, "a.png", 8)).unwrap_err(),
            "preview_binary_format_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_reserved_paths_missing_materialization_and_oversize() {
        let root =
            std::env::temp_dir().join(format!("cilai-binary-preview-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let mut placeholder = record(&root, "a.png", 0);
        placeholder.materialization_state = "placeholder".to_owned();
        assert_eq!(
            read(&placeholder).unwrap_err(),
            "preview_file_not_materialized"
        );
        for reserved in [".cilai", ".CILAI", ".contexthub", "nested/.cilai"] {
            assert_eq!(
                read(&record(&root, &format!("{reserved}/a.png"), 0)).unwrap_err(),
                "preview_permission_denied"
            );
        }
        fs::File::create(root.join("large.png"))
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert_eq!(
            read(&record(
                &root,
                "large.png",
                i64::try_from(MAX_BYTES + 1).unwrap()
            ))
            .unwrap_err(),
            "preview_file_too_large"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
