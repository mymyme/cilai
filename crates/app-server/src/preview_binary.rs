use base64::Engine as _;
use contexthub_application::PreviewFileRecord;
use contexthub_domain::WorkspaceRelativePath;
use contexthub_platform_adapter::filesystem as fs;
use contexthub_protocol::ReadBinaryPreviewResult;
use contexthub_workspace_fs::resolve_existing_workspace_path;
use sha2::{Digest, Sha256};
use std::io::Read as _;
use std::path::Path;

const MAX_BYTES: u64 = 32 * 1024 * 1024;

pub(super) fn read(record: &PreviewFileRecord) -> Result<ReadBinaryPreviewResult, &'static str> {
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
    Ok(ReadBinaryPreviewResult {
        content_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
        content_revision: revision,
        content_type: content_type.to_owned(),
        size_bytes: bytes.len() as u64,
    })
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
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(result.content_base64)
                .unwrap(),
            bytes
        );
        let mut changed = source.clone();
        changed.checksum_algorithm = Some("sha256".to_owned());
        changed.checksum_value = Some("0".repeat(64));
        assert_eq!(read(&changed).unwrap_err(), "preview_source_changed");
        fs::write(root.join("a.png"), b"<script>").unwrap();
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
