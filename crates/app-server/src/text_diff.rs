use crate::text_resource::{TextResourceTickets, TextTicketError};
use contexthub_protocol::{
    ComputeTextDiffParams, ComputeTextDiffResult, DiffSourceRef, ReadTextDiffResult, TextDiffRow,
    TextDiffRowKind, TextDiffSummary,
};
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;

pub const DEFAULT_MAX_DIFF_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
pub const DEFAULT_MAX_DIFF_TICKETS: usize = 32;
pub const DEFAULT_DIFF_TICKET_TTL_MILLIS: i64 = 5 * 60 * 1000;
pub const MAX_DIFF_ROWS: usize = 200_000;
pub const MAX_DIFF_LINE_BYTES: usize = 60 * 1024;
pub const MAX_DIFF_PAGE_ROWS: u32 = 200;
pub const MIN_DIFF_PAGE_BYTES: u32 = 64 * 1024;
pub const MAX_DIFF_PAGE_BYTES: u32 = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextDiffError {
    InvalidRequest,
    CapacityReached,
    SourceTooLarge,
    TooManyRows,
    LineTooLarge,
    TicketNotFound,
    TicketExpired,
    SourceChanged,
    ReadFailed,
}

impl TextDiffError {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::InvalidRequest => "text_diff_invalid_request",
            Self::CapacityReached => "text_diff_capacity_reached",
            Self::SourceTooLarge => "text_diff_source_too_large",
            Self::TooManyRows => "text_diff_too_many_rows",
            Self::LineTooLarge => "text_diff_line_too_large",
            Self::TicketNotFound => "text_diff_ticket_not_found",
            Self::TicketExpired => "text_diff_ticket_expired",
            Self::SourceChanged => "text_diff_source_changed",
            Self::ReadFailed => "text_diff_read_failed",
        }
    }
}

struct DiffEntry {
    expires_at: i64,
    left: DiffSourceRef,
    right: DiffSourceRef,
    rows: Vec<TextDiffRow>,
}

pub struct TextDiffTickets {
    entries: HashMap<String, DiffEntry>,
    max_tickets: usize,
    ttl_millis: i64,
    max_source_bytes: u64,
}

impl Default for TextDiffTickets {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            max_tickets: DEFAULT_MAX_DIFF_TICKETS,
            ttl_millis: DEFAULT_DIFF_TICKET_TTL_MILLIS,
            max_source_bytes: DEFAULT_MAX_DIFF_SOURCE_BYTES,
        }
    }
}

impl TextDiffTickets {
    /// 读取两个固定 revision 的文本资源并建立有界、分页的逐行差异票据。
    ///
    /// # Errors
    ///
    /// 参数、资源票据、revision、容量或输出边界不合法，以及任一来源变化时返回错误。
    pub fn compute(
        &mut self,
        resources: &mut TextResourceTickets,
        diff_ticket_id: String,
        params: &ComputeTextDiffParams,
        now: i64,
    ) -> Result<ComputeTextDiffResult, TextDiffError> {
        self.evict_expired(now);
        validate_compute_request(&diff_ticket_id, params, now)?;
        if self.entries.len() >= self.max_tickets {
            return Err(TextDiffError::CapacityReached);
        }
        let left = resources
            .read_all_verified(
                &params.left.ticket_id,
                &params.left.content_revision,
                self.max_source_bytes,
                now,
            )
            .map_err(map_resource_error)?;
        let right = resources
            .read_all_verified(
                &params.right.ticket_id,
                &params.right.content_revision,
                self.max_source_bytes,
                now,
            )
            .map_err(map_resource_error)?;
        resources
            .verify(&params.left.ticket_id, &params.left.content_revision, now)
            .map_err(map_resource_error)?;
        resources
            .verify(&params.right.ticket_id, &params.right.content_revision, now)
            .map_err(map_resource_error)?;
        let (rows, summary) = compute_rows(&left, &right)?;
        let total_rows = u32::try_from(rows.len()).map_err(|_| TextDiffError::TooManyRows)?;
        let expires_at = now
            .checked_add(self.ttl_millis)
            .ok_or(TextDiffError::InvalidRequest)?;
        self.entries.insert(
            diff_ticket_id.clone(),
            DiffEntry {
                expires_at,
                left: params.left.clone(),
                right: params.right.clone(),
                rows,
            },
        );
        Ok(ComputeTextDiffResult {
            diff_ticket_id,
            expires_at,
            left: params.left.clone(),
            right: params.right.clone(),
            summary,
            total_rows,
        })
    }

    /// # Errors
    ///
    /// 差异票据不存在、过期、分页参数非法或任一源 revision 已失效时返回错误。
    pub fn read(
        &mut self,
        resources: &mut TextResourceTickets,
        diff_ticket_id: &str,
        cursor: u32,
        max_rows: u32,
        max_bytes: u32,
        now: i64,
    ) -> Result<ReadTextDiffResult, TextDiffError> {
        if max_rows == 0
            || max_rows > MAX_DIFF_PAGE_ROWS
            || !(MIN_DIFF_PAGE_BYTES..=MAX_DIFF_PAGE_BYTES).contains(&max_bytes)
        {
            return Err(TextDiffError::InvalidRequest);
        }
        let Some(entry) = self.entries.get(diff_ticket_id) else {
            return Err(TextDiffError::TicketNotFound);
        };
        if now < 0 || now >= entry.expires_at {
            self.entries.remove(diff_ticket_id);
            return Err(TextDiffError::TicketExpired);
        }
        let start = usize::try_from(cursor).map_err(|_| TextDiffError::InvalidRequest)?;
        if start > entry.rows.len() {
            return Err(TextDiffError::InvalidRequest);
        }
        if let Err(error) = verify_sources(resources, entry, now) {
            self.entries.remove(diff_ticket_id);
            return Err(error);
        }
        let mut rows = Vec::new();
        let mut used_bytes = 0_usize;
        let max_bytes = usize::try_from(max_bytes).map_err(|_| TextDiffError::InvalidRequest)?;
        let max_rows = usize::try_from(max_rows).map_err(|_| TextDiffError::InvalidRequest)?;
        for row in entry.rows.iter().skip(start).take(max_rows) {
            let row_bytes = row.text.len().saturating_add(64);
            if !rows.is_empty() && used_bytes.saturating_add(row_bytes) > max_bytes {
                break;
            }
            if row_bytes > max_bytes {
                return Err(TextDiffError::LineTooLarge);
            }
            used_bytes = used_bytes.saturating_add(row_bytes);
            rows.push(row.clone());
        }
        let end = start
            .checked_add(rows.len())
            .ok_or(TextDiffError::InvalidRequest)?;
        let next_cursor = (end < entry.rows.len())
            .then(|| u32::try_from(end).map_err(|_| TextDiffError::TooManyRows))
            .transpose()?;
        Ok(ReadTextDiffResult {
            cursor,
            next_cursor,
            rows,
        })
    }

    fn evict_expired(&mut self, now: i64) {
        self.entries.retain(|_, entry| entry.expires_at > now);
    }
}

fn validate_compute_request(
    diff_ticket_id: &str,
    params: &ComputeTextDiffParams,
    now: i64,
) -> Result<(), TextDiffError> {
    if diff_ticket_id.is_empty()
        || now < 0
        || params.left.ticket_id.is_empty()
        || params.right.ticket_id.is_empty()
        || params.left.content_revision.is_empty()
        || params.right.content_revision.is_empty()
        || params.left.label.is_empty()
        || params.right.label.is_empty()
        || params.left.label.len() > 256
        || params.right.label.len() > 256
        || params
            .left
            .version
            .as_ref()
            .is_some_and(|value| value.len() > 256)
        || params
            .right
            .version
            .as_ref()
            .is_some_and(|value| value.len() > 256)
    {
        return Err(TextDiffError::InvalidRequest);
    }
    Ok(())
}

fn compute_rows(
    left: &str,
    right: &str,
) -> Result<(Vec<TextDiffRow>, TextDiffSummary), TextDiffError> {
    let diff = TextDiff::from_lines(left, right);
    let mut rows = Vec::new();
    let mut summary = TextDiffSummary {
        added_lines: 0,
        deleted_lines: 0,
        unchanged_lines: 0,
        change_blocks: 0,
    };
    let mut in_change_block = false;
    for change in diff.iter_all_changes() {
        if rows.len() >= MAX_DIFF_ROWS {
            return Err(TextDiffError::TooManyRows);
        }
        let kind = match change.tag() {
            ChangeTag::Equal => {
                in_change_block = false;
                summary.unchanged_lines = increment(summary.unchanged_lines)?;
                TextDiffRowKind::Equal
            }
            ChangeTag::Delete => {
                if !in_change_block {
                    summary.change_blocks = increment(summary.change_blocks)?;
                    in_change_block = true;
                }
                summary.deleted_lines = increment(summary.deleted_lines)?;
                TextDiffRowKind::Delete
            }
            ChangeTag::Insert => {
                if !in_change_block {
                    summary.change_blocks = increment(summary.change_blocks)?;
                    in_change_block = true;
                }
                summary.added_lines = increment(summary.added_lines)?;
                TextDiffRowKind::Insert
            }
        };
        let (text, has_line_terminator) = line_content(change.value());
        if text.len() > MAX_DIFF_LINE_BYTES {
            return Err(TextDiffError::LineTooLarge);
        }
        rows.push(TextDiffRow {
            kind,
            left_line: line_number(change.old_index())?,
            right_line: line_number(change.new_index())?,
            text,
            has_line_terminator,
        });
    }
    Ok((rows, summary))
}

fn verify_sources(
    resources: &mut TextResourceTickets,
    entry: &DiffEntry,
    now: i64,
) -> Result<(), TextDiffError> {
    resources
        .verify(&entry.left.ticket_id, &entry.left.content_revision, now)
        .map_err(map_resource_error)?;
    resources
        .verify(&entry.right.ticket_id, &entry.right.content_revision, now)
        .map_err(map_resource_error)
}

fn line_content(value: &str) -> (String, bool) {
    if let Some(text) = value.strip_suffix("\r\n") {
        (text.to_owned(), true)
    } else if let Some(text) = value.strip_suffix('\n') {
        (text.to_owned(), true)
    } else {
        (value.to_owned(), false)
    }
}

fn line_number(index: Option<usize>) -> Result<Option<u32>, TextDiffError> {
    index
        .map(|value| {
            value
                .checked_add(1)
                .and_then(|line| u32::try_from(line).ok())
                .ok_or(TextDiffError::TooManyRows)
        })
        .transpose()
}

fn increment(value: u32) -> Result<u32, TextDiffError> {
    value.checked_add(1).ok_or(TextDiffError::TooManyRows)
}

fn map_resource_error(error: TextTicketError) -> TextDiffError {
    match error {
        TextTicketError::InvalidRequest => TextDiffError::InvalidRequest,
        TextTicketError::CapacityReached => TextDiffError::CapacityReached,
        TextTicketError::ResourceTooLarge => TextDiffError::SourceTooLarge,
        TextTicketError::NotFound => TextDiffError::TicketNotFound,
        TextTicketError::Expired => TextDiffError::TicketExpired,
        TextTicketError::Source(source) => match source.kind() {
            contexthub_workspace_fs::TextResourceErrorKind::SourceChanged => {
                TextDiffError::SourceChanged
            }
            _ => TextDiffError::ReadFailed,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_protocol::PreviewSource;
    use contexthub_workspace_fs::ExpectedTextResourceRevision;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn computes_summary_rows_and_invalidates_when_a_source_changes() {
        let root = test_root();
        fs::write(root.join("left.md"), "one\ntwo\nkeep\n").unwrap();
        fs::write(root.join("right.md"), "one\nchanged\nkeep\nnew").unwrap();
        let mut resources = TextResourceTickets::default();
        let left = issue(&mut resources, &root, "left", "left.md", 13);
        let right = issue(&mut resources, &root, "right", "right.md", 20);
        let params = ComputeTextDiffParams {
            left: source(
                "left",
                PreviewSource::Baseline,
                &left.metadata.content_revision,
            ),
            right: source(
                "right",
                PreviewSource::Local,
                &right.metadata.content_revision,
            ),
        };
        let mut diffs = TextDiffTickets::default();
        let result = diffs
            .compute(&mut resources, "diff-1".to_owned(), &params, 2)
            .unwrap();
        assert_eq!(result.summary.added_lines, 2);
        assert_eq!(result.summary.deleted_lines, 1);
        assert_eq!(result.summary.unchanged_lines, 2);
        assert_eq!(result.summary.change_blocks, 2);
        let page = diffs
            .read(&mut resources, "diff-1", 0, 2, MIN_DIFF_PAGE_BYTES, 3)
            .unwrap();
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.next_cursor, Some(2));

        fs::write(root.join("left.md"), "replaced").unwrap();
        assert_eq!(
            diffs
                .read(&mut resources, "diff-1", 2, 200, MIN_DIFF_PAGE_BYTES, 4)
                .unwrap_err(),
            TextDiffError::SourceChanged
        );
        assert_eq!(
            diffs
                .read(&mut resources, "diff-1", 0, 200, MIN_DIFF_PAGE_BYTES, 4)
                .unwrap_err(),
            TextDiffError::TicketNotFound
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn issue(
        resources: &mut TextResourceTickets,
        root: &std::path::Path,
        id: &str,
        path: &str,
        size: u64,
    ) -> crate::text_resource::TextResourceTicket {
        resources
            .issue(
                id.to_owned(),
                root,
                path,
                &ExpectedTextResourceRevision {
                    size_bytes: Some(size),
                    checksum_algorithm: None,
                    checksum_value: None,
                },
                1,
            )
            .unwrap()
    }

    fn source(id: &str, source: PreviewSource, revision: &str) -> DiffSourceRef {
        DiffSourceRef {
            ticket_id: id.to_owned(),
            source,
            content_revision: revision.to_owned(),
            label: format!("{id}.md"),
            version: None,
            modified_at: Some(1),
        }
    }

    fn test_root() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ch-diff-{}-{nanos}", std::process::id()));
        fs::create_dir(&root).unwrap();
        root
    }
}
