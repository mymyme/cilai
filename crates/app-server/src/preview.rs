use crate::text_resource::{TextResourceTickets, TextTicketError};
use contexthub_application::PreviewFileRecord;
use contexthub_protocol::{
    OpenPreviewResult, PreviewAvailability, PreviewFormat, PreviewSource, ReadTextResourceResult,
};
use contexthub_workspace_fs::{ExpectedTextResourceRevision, TextResourceErrorKind};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewError {
    InvalidRequest,
    FileMissing,
    FileNotMaterialized,
    FileNotRegular,
    TicketCapacityReached,
    TicketNotFound,
    TicketExpired,
    FileTooLarge,
    BinaryContent,
    UnsupportedEncoding,
    SourceChanged,
    PermissionDenied,
    ReadFailed,
}

impl PreviewError {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_preview_request",
            Self::FileMissing => "preview_file_missing",
            Self::FileNotMaterialized => "preview_file_not_materialized",
            Self::FileNotRegular => "preview_file_not_regular",
            Self::TicketCapacityReached => "preview_ticket_capacity_reached",
            Self::TicketNotFound => "preview_ticket_not_found",
            Self::TicketExpired => "preview_ticket_expired",
            Self::FileTooLarge => "preview_file_too_large",
            Self::BinaryContent => "preview_binary_content",
            Self::UnsupportedEncoding => "preview_encoding_unsupported",
            Self::SourceChanged => "preview_source_changed",
            Self::PermissionDenied => "preview_permission_denied",
            Self::ReadFailed => "preview_read_failed",
        }
    }
}

/// 为当前文件观察创建预览描述。只有本地来源签发文本资源票据；尚未物化的
/// 基线与平台 Mock 保持显式不可用，不能借用本地正文伪装来源。
///
/// # Errors
///
/// 文件记录、大小、资源签发或源内容验证失败时返回稳定错误类别。
#[allow(clippy::too_many_lines)]
pub fn open_preview(
    tickets: &mut TextResourceTickets,
    record: &PreviewFileRecord,
    source: PreviewSource,
    ticket_id: String,
    now: i64,
) -> Result<OpenPreviewResult, PreviewError> {
    validate_record(record)?;
    let size_bytes = record
        .size_bytes
        .map(u64::try_from)
        .transpose()
        .map_err(|_| PreviewError::InvalidRequest)?;
    let format = classify_preview_format(&record.relative_path);
    if format == PreviewFormat::Unsupported {
        return Ok(descriptor(
            record,
            source,
            PreviewAvailability::Unsupported,
            format,
            size_bytes,
            Some("preview_format_unsupported".to_owned()),
        ));
    }
    if source == PreviewSource::Baseline {
        if !matches!(
            format,
            PreviewFormat::Markdown | PreviewFormat::Html | PreviewFormat::PlainText
        ) {
            return Ok(descriptor(
                record,
                source,
                PreviewAvailability::Unavailable,
                format,
                size_bytes,
                Some("preview_baseline_not_materialized".to_owned()),
            ));
        }
        let baseline = record
            .baseline_text_content
            .clone()
            .or_else(|| (record.local_change_state.as_deref() == Some("added")).then(String::new));
        let Some(content) = baseline else {
            return Ok(descriptor(
                record,
                source,
                PreviewAvailability::Unavailable,
                format,
                size_bytes,
                Some("preview_baseline_not_materialized".to_owned()),
            ));
        };
        let ticket = tickets
            .issue_memory(
                ticket_id,
                content,
                record.baseline_content_revision.clone(),
                now,
            )
            .map_err(map_ticket_error)?;
        return Ok(OpenPreviewResult {
            source,
            availability: PreviewAvailability::Ready,
            format,
            relative_path: record.relative_path.clone(),
            size_bytes: Some(ticket.metadata.size_bytes),
            source_modified_at: record.baseline_confirmed_at,
            content_revision: Some(ticket.metadata.content_revision.clone()),
            content_bytes: Some(ticket.metadata.content_bytes),
            ticket_id: Some(ticket.ticket_id),
            ticket_expires_at: Some(ticket.expires_at),
            reason: None,
        });
    }
    if source != PreviewSource::Local {
        return Ok(descriptor(
            record,
            source,
            PreviewAvailability::Unavailable,
            format,
            size_bytes,
            Some("preview_source_not_materialized".to_owned()),
        ));
    }
    if matches!(
        format,
        PreviewFormat::Pdf
            | PreviewFormat::Image
            | PreviewFormat::Docx
            | PreviewFormat::Pptx
            | PreviewFormat::Xlsx
            | PreviewFormat::Xmind
    ) {
        return Ok(descriptor(
            record,
            source,
            PreviewAvailability::Ready,
            format,
            size_bytes,
            None,
        ));
    }
    let expected = ExpectedTextResourceRevision {
        size_bytes,
        checksum_algorithm: record.checksum_algorithm.clone(),
        checksum_value: record.checksum_value.clone(),
    };
    let ticket = tickets
        .issue(
            ticket_id,
            Path::new(&record.workspace_root),
            &record.relative_path,
            &expected,
            now,
        )
        .map_err(map_ticket_error)?;
    Ok(OpenPreviewResult {
        source,
        availability: PreviewAvailability::Ready,
        format,
        relative_path: record.relative_path.clone(),
        size_bytes: Some(ticket.metadata.size_bytes),
        source_modified_at: record.modified_at_fs,
        content_revision: Some(ticket.metadata.content_revision.clone()),
        content_bytes: Some(ticket.metadata.content_bytes),
        ticket_id: Some(ticket.ticket_id),
        ticket_expires_at: Some(ticket.expires_at),
        reason: None,
    })
}

/// # Errors
///
/// 票据无效、过期、源变化或文本读取失败时返回稳定错误类别。
pub fn read_preview(
    tickets: &mut TextResourceTickets,
    ticket_id: &str,
    offset: u64,
    max_bytes: u32,
    now: i64,
) -> Result<ReadTextResourceResult, PreviewError> {
    let max_bytes = usize::try_from(max_bytes).map_err(|_| PreviewError::InvalidRequest)?;
    let chunk = tickets
        .read(ticket_id, offset, max_bytes, now)
        .map_err(map_ticket_error)?;
    Ok(ReadTextResourceResult {
        offset: chunk.offset,
        next_offset: chunk
            .next_offset
            .unwrap_or(chunk.offset + chunk.text.len() as u64),
        end_of_resource: chunk.next_offset.is_none(),
        content_revision: chunk.content_revision,
        text: chunk.text,
    })
}

#[must_use]
pub fn classify_preview_format(relative_path: &str) -> PreviewFormat {
    let extension = Path::new(relative_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("md" | "markdown" | "mkd") => PreviewFormat::Markdown,
        // 单独成一类，界面才能在"预览"下渲染它、在"源码"下显示原文。
        // 渲染一律走禁用脚本的沙箱内联框架，不进主文档。
        Some("html" | "htm") => PreviewFormat::Html,
        // PDF 保持二进制形态，由桌面宿主在工作区边界内读取并交给 WebView 的原生
        // PDF 查看器；不能复用 UTF-8 文本票据，否则会被误判为二进制文本错误。
        Some("pdf") => PreviewFormat::Pdf,
        Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp") => PreviewFormat::Image,
        Some("docx") => PreviewFormat::Docx,
        Some("pptx") => PreviewFormat::Pptx,
        Some("xlsx") => PreviewFormat::Xlsx,
        Some("xmind") => PreviewFormat::Xmind,
        Some(
            "txt" | "log" | "csv" | "tsv" | "json" | "jsonl" | "yaml" | "yml" | "toml" | "xml"
            | "ini" | "conf" | "properties" | "sql" | "sh" | "bash" | "zsh" | "rs" | "ts" | "tsx"
            | "js" | "jsx" | "py" | "go" | "java" | "kt" | "kts" | "swift" | "c" | "h" | "cc"
            | "cpp" | "hpp" | "cs" | "css" | "scss",
        ) => PreviewFormat::PlainText,
        _ => PreviewFormat::Unsupported,
    }
}

fn descriptor(
    record: &PreviewFileRecord,
    source: PreviewSource,
    availability: PreviewAvailability,
    format: PreviewFormat,
    size_bytes: Option<u64>,
    reason: Option<String>,
) -> OpenPreviewResult {
    OpenPreviewResult {
        source,
        availability,
        format,
        relative_path: record.relative_path.clone(),
        size_bytes,
        source_modified_at: record.modified_at_fs,
        content_revision: None,
        content_bytes: None,
        ticket_id: None,
        ticket_expires_at: None,
        reason,
    }
}

fn validate_record(record: &PreviewFileRecord) -> Result<(), PreviewError> {
    if record.workspace_root.is_empty()
        || record.relative_path.is_empty()
        || record.workspace_id.as_str().is_empty()
        || record.local_file_id.is_empty()
    {
        return Err(PreviewError::InvalidRequest);
    }
    if record.materialization_state == "missing" {
        return Err(PreviewError::FileMissing);
    }
    if record.materialization_state != "materialized" {
        return Err(PreviewError::FileNotMaterialized);
    }
    if record.file_type != "file" {
        return Err(PreviewError::FileNotRegular);
    }
    Ok(())
}

fn map_ticket_error(error: TextTicketError) -> PreviewError {
    match error {
        TextTicketError::InvalidRequest => PreviewError::InvalidRequest,
        TextTicketError::CapacityReached => PreviewError::TicketCapacityReached,
        TextTicketError::ResourceTooLarge => PreviewError::FileTooLarge,
        TextTicketError::NotFound => PreviewError::TicketNotFound,
        TextTicketError::Expired => PreviewError::TicketExpired,
        TextTicketError::Source(source) => match source.kind() {
            TextResourceErrorKind::FileTooLarge => PreviewError::FileTooLarge,
            TextResourceErrorKind::BinaryContent => PreviewError::BinaryContent,
            TextResourceErrorKind::UnsupportedEncoding => PreviewError::UnsupportedEncoding,
            TextResourceErrorKind::SourceChanged => PreviewError::SourceChanged,
            TextResourceErrorKind::SourceNotMaterialized => PreviewError::FileNotMaterialized,
            TextResourceErrorKind::TargetUnavailable => PreviewError::FileMissing,
            TextResourceErrorKind::RootUnavailable
            | TextResourceErrorKind::OutsideWorkspace
            | TextResourceErrorKind::ReservedPath
            | TextResourceErrorKind::NotRegularFile
            | TextResourceErrorKind::InvalidOffset
            | TextResourceErrorKind::InvalidChunkLimit => PreviewError::InvalidRequest,
            TextResourceErrorKind::ReadFailed => PreviewError::ReadFailed,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_domain::WorkspaceId;
    use std::fs;

    #[test]
    fn local_markdown_and_new_file_empty_baseline_use_distinct_tickets() {
        let root = test_root();
        fs::write(root.join("note.md"), "# Title\n\n正文").unwrap();
        let file_record = record(&root, "note.md", 15);
        let mut tickets = TextResourceTickets::default();
        let opened = open_preview(
            &mut tickets,
            &file_record,
            PreviewSource::Local,
            "ticket-1".to_owned(),
            1,
        )
        .unwrap();
        assert_eq!(opened.availability, PreviewAvailability::Ready);
        assert_eq!(opened.format, PreviewFormat::Markdown);
        assert_eq!(opened.source, PreviewSource::Local);
        let content = read_preview(&mut tickets, "ticket-1", 0, 256, 2).unwrap();
        assert_eq!(content.text, "# Title\n\n正文");
        assert!(content.end_of_resource);

        let baseline = open_preview(
            &mut tickets,
            &file_record,
            PreviewSource::Baseline,
            "unused".to_owned(),
            2,
        )
        .unwrap();
        assert_eq!(baseline.availability, PreviewAvailability::Ready);
        assert_eq!(baseline.source, PreviewSource::Baseline);
        let baseline_content = read_preview(&mut tickets, "unused", 0, 256, 3).unwrap();
        assert_eq!(baseline_content.text, "");

        let mut confirmed = record(&root, "note.md", 15);
        confirmed.local_change_state = Some("modified".to_owned());
        confirmed.baseline_text_content = Some("# Old".to_owned());
        confirmed.baseline_content_revision = Some("a".repeat(64));
        let old = open_preview(
            &mut tickets,
            &confirmed,
            PreviewSource::Baseline,
            "confirmed-baseline".to_owned(),
            4,
        )
        .unwrap();
        assert_eq!(old.availability, PreviewAvailability::Ready);
        assert_eq!(
            read_preview(&mut tickets, "confirmed-baseline", 0, 256, 5)
                .unwrap()
                .text,
            "# Old"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn binary_baseline_never_uses_current_local_bytes_or_an_empty_text_ticket() {
        let root = test_root();
        fs::write(root.join("slides.pptx"), b"PK\x03\x04").unwrap();
        let result = open_preview(
            &mut TextResourceTickets::default(),
            &record(&root, "slides.pptx", 4),
            PreviewSource::Baseline,
            "baseline-ticket".to_owned(),
            1,
        )
        .unwrap();
        assert_eq!(result.availability, PreviewAvailability::Unavailable);
        assert!(result.ticket_id.is_none());
        assert_eq!(result.source, PreviewSource::Baseline);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_and_read_failures_remain_distinct() {
        let root = test_root();
        fs::write(root.join("archive.bin"), [1_u8, 2, 3]).unwrap();
        let unsupported = open_preview(
            &mut TextResourceTickets::default(),
            &record(&root, "archive.bin", 3),
            PreviewSource::PlatformMock,
            "unused".to_owned(),
            1,
        )
        .unwrap();
        assert_eq!(unsupported.availability, PreviewAvailability::Unsupported);
        assert_eq!(
            unsupported.reason.as_deref(),
            Some("preview_format_unsupported")
        );

        fs::write(root.join("bad.txt"), [0_u8, 1, 2]).unwrap();
        let error = open_preview(
            &mut TextResourceTickets::default(),
            &record(&root, "bad.txt", 3),
            PreviewSource::Local,
            "ticket-bad".to_owned(),
            1,
        )
        .unwrap_err();
        assert_eq!(error, PreviewError::BinaryContent);
        fs::remove_dir_all(root).unwrap();
    }

    fn record(root: &Path, relative_path: &str, size_bytes: i64) -> PreviewFileRecord {
        PreviewFileRecord {
            workspace_id: WorkspaceId::new("workspace-preview"),
            local_file_id: "file-preview".to_owned(),
            workspace_root: root.to_string_lossy().into_owned(),
            relative_path: relative_path.to_owned(),
            materialization_state: "materialized".to_owned(),
            file_type: "file".to_owned(),
            size_bytes: Some(size_bytes),
            modified_at_fs: Some(1_700_000_000_000),
            checksum_algorithm: None,
            checksum_value: None,
            local_change_state: Some("added".to_owned()),
            baseline_text_content: None,
            baseline_content_revision: None,
            baseline_confirmed_at: None,
        }
    }

    fn test_root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("ch-preview-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn html_is_its_own_format_so_the_ui_can_render_or_show_source() {
        // 归到纯文本就只能看源码，界面无从提供渲染视图。
        assert_eq!(classify_preview_format("page.html"), PreviewFormat::Html);
        assert_eq!(classify_preview_format("Page.HTM"), PreviewFormat::Html);
        assert_eq!(classify_preview_format("notes.md"), PreviewFormat::Markdown);
        assert_eq!(classify_preview_format("contract.PDF"), PreviewFormat::Pdf);
        assert_eq!(
            classify_preview_format("style.css"),
            PreviewFormat::PlainText
        );
        assert_eq!(classify_preview_format("photo.png"), PreviewFormat::Image);
    }
}
