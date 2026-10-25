use super::CaptureLocalSnapshotError;
use contexthub_persistence_sqlite::FileSearchContent;
use contexthub_workspace_fs::{
    ExpectedTextResourceRevision, MAX_TEXT_CHUNK_BYTES, TextResourceErrorKind, WorkspaceScanEntry,
    open_text_resource,
};
use std::path::Path;

pub(super) const MAX_SEARCH_TEXT_BYTES: u64 = 2 * 1024 * 1024;

pub(super) fn visible_path(entry: &WorkspaceScanEntry) -> bool {
    !entry
        .path
        .as_str()
        .split('/')
        .any(|part| part.starts_with('.'))
}

pub(super) fn read(
    root: &str,
    entry: &WorkspaceScanEntry,
    checksum: &str,
) -> Result<FileSearchContent, CaptureLocalSnapshotError> {
    let expected = ExpectedTextResourceRevision {
        size_bytes: entry.byte_len,
        checksum_algorithm: Some("sha256".to_owned()),
        checksum_value: Some(checksum.to_owned()),
    };
    let mut resource = match open_text_resource(
        Path::new(root),
        &entry.path,
        &expected,
        MAX_SEARCH_TEXT_BYTES,
    ) {
        Ok(resource) => resource,
        Err(error) => {
            return match error.kind() {
                TextResourceErrorKind::BinaryContent => Ok(FileSearchContent::Binary),
                TextResourceErrorKind::UnsupportedEncoding => {
                    Ok(FileSearchContent::UnsupportedEncoding)
                }
                TextResourceErrorKind::FileTooLarge => Ok(FileSearchContent::TooLarge),
                _ => Err(CaptureLocalSnapshotError::TextIndex(error)),
            };
        }
    };
    let mut text = String::new();
    let mut offset = resource.metadata().first_offset;
    loop {
        let chunk = resource
            .read_chunk(offset, MAX_TEXT_CHUNK_BYTES)
            .map_err(CaptureLocalSnapshotError::TextIndex)?;
        text.push_str(&chunk.text);
        let Some(next) = chunk.next_offset else { break };
        offset = next;
    }
    resource
        .verify_unchanged()
        .map_err(CaptureLocalSnapshotError::TextIndex)?;
    Ok(FileSearchContent::Text(text))
}
