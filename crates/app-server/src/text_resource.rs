use contexthub_domain::WorkspaceRelativePath;
use contexthub_workspace_fs::{
    DEFAULT_MAX_TEXT_RESOURCE_BYTES, ExpectedTextResourceRevision, MAX_TEXT_CHUNK_BYTES,
    MIN_TEXT_CHUNK_BYTES, OpenedTextResource, TextEncoding, TextResourceChunk, TextResourceError,
    TextResourceMetadata, open_text_resource,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

pub const DEFAULT_TEXT_TICKET_TTL_MILLIS: i64 = 5 * 60 * 1000;
pub const DEFAULT_MAX_TEXT_TICKETS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextResourceTicket {
    pub ticket_id: String,
    pub expires_at: i64,
    pub metadata: TextResourceMetadata,
}

#[derive(Debug)]
pub enum TextTicketError {
    InvalidRequest,
    CapacityReached,
    ResourceTooLarge,
    NotFound,
    Expired,
    Source(TextResourceError),
}

struct TicketEntry {
    expires_at: i64,
    resource: TicketResource,
}

enum TicketResource {
    File(OpenedTextResource),
    Memory {
        content: String,
        metadata: TextResourceMetadata,
    },
}

impl TicketResource {
    fn metadata(&self) -> &TextResourceMetadata {
        match self {
            Self::File(resource) => resource.metadata(),
            Self::Memory { metadata, .. } => metadata,
        }
    }

    fn verify_unchanged(&self) -> Result<(), TextTicketError> {
        match self {
            Self::File(resource) => resource.verify_unchanged().map_err(TextTicketError::Source),
            Self::Memory { .. } => Ok(()),
        }
    }

    fn read_chunk(
        &mut self,
        offset: u64,
        max_bytes: usize,
    ) -> Result<TextResourceChunk, TextTicketError> {
        match self {
            Self::File(resource) => resource
                .read_chunk(offset, max_bytes)
                .map_err(TextTicketError::Source),
            Self::Memory { content, metadata } => {
                if !(MIN_TEXT_CHUNK_BYTES..=MAX_TEXT_CHUNK_BYTES).contains(&max_bytes) {
                    return Err(TextTicketError::InvalidRequest);
                }
                let start = usize::try_from(offset).map_err(|_| TextTicketError::InvalidRequest)?;
                if start > content.len() || !content.is_char_boundary(start) {
                    return Err(TextTicketError::InvalidRequest);
                }
                if start == content.len() {
                    return Ok(TextResourceChunk {
                        text: String::new(),
                        offset,
                        next_offset: None,
                        content_revision: metadata.content_revision.clone(),
                    });
                }
                let mut end = start.saturating_add(max_bytes).min(content.len());
                while end > start && !content.is_char_boundary(end) {
                    end -= 1;
                }
                if end == start {
                    return Err(TextTicketError::InvalidRequest);
                }
                Ok(TextResourceChunk {
                    text: content[start..end].to_owned(),
                    offset,
                    next_offset: (end < content.len()).then_some(
                        u64::try_from(end).map_err(|_| TextTicketError::InvalidRequest)?,
                    ),
                    content_revision: metadata.content_revision.clone(),
                })
            }
        }
    }
}

pub struct TextResourceTickets {
    entries: HashMap<String, TicketEntry>,
    max_tickets: usize,
    ttl_millis: i64,
    max_resource_bytes: u64,
}

impl Default for TextResourceTickets {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_TEXT_TICKETS,
            DEFAULT_TEXT_TICKET_TTL_MILLIS,
            DEFAULT_MAX_TEXT_RESOURCE_BYTES,
        )
    }
}

impl TextResourceTickets {
    #[must_use]
    pub fn new(max_tickets: usize, ttl_millis: i64, max_resource_bytes: u64) -> Self {
        Self {
            entries: HashMap::new(),
            max_tickets,
            ttl_millis,
            max_resource_bytes,
        }
    }

    /// 签发一个仅能访问已验证相对路径和固定内容 revision 的进程内票据。
    ///
    /// # Errors
    ///
    /// 请求、容量、路径、编码、大小或期望 revision 不合法时返回错误。
    pub fn issue(
        &mut self,
        ticket_id: String,
        workspace_root: &Path,
        relative_path: &str,
        expected: &ExpectedTextResourceRevision,
        now: i64,
    ) -> Result<TextResourceTicket, TextTicketError> {
        self.evict_expired(now);
        if ticket_id.is_empty() || now < 0 || self.ttl_millis <= 0 || self.max_tickets == 0 {
            return Err(TextTicketError::InvalidRequest);
        }
        if self.entries.contains_key(&ticket_id) {
            return Err(TextTicketError::InvalidRequest);
        }
        if self.entries.len() >= self.max_tickets {
            return Err(TextTicketError::CapacityReached);
        }
        let path = WorkspaceRelativePath::parse(relative_path)
            .map_err(|_| TextTicketError::InvalidRequest)?;
        let resource = open_text_resource(workspace_root, &path, expected, self.max_resource_bytes)
            .map_err(TextTicketError::Source)?;
        let expires_at = now
            .checked_add(self.ttl_millis)
            .ok_or(TextTicketError::InvalidRequest)?;
        let metadata = resource.metadata().clone();
        self.entries.insert(
            ticket_id.clone(),
            TicketEntry {
                expires_at,
                resource: TicketResource::File(resource),
            },
        );
        Ok(TextResourceTicket {
            ticket_id,
            expires_at,
            metadata,
        })
    }

    /// 为已持久化、不可变的确认快照签发进程内文本票据。
    ///
    /// # Errors
    ///
    /// 票据参数无效、容量已满或内容超过资源大小上限时返回错误。
    pub fn issue_memory(
        &mut self,
        ticket_id: String,
        content: String,
        content_revision: Option<String>,
        now: i64,
    ) -> Result<TextResourceTicket, TextTicketError> {
        self.evict_expired(now);
        if ticket_id.is_empty()
            || now < 0
            || self.ttl_millis <= 0
            || self.max_tickets == 0
            || self.entries.contains_key(&ticket_id)
        {
            return Err(TextTicketError::InvalidRequest);
        }
        if self.entries.len() >= self.max_tickets {
            return Err(TextTicketError::CapacityReached);
        }
        let size_bytes =
            u64::try_from(content.len()).map_err(|_| TextTicketError::InvalidRequest)?;
        if size_bytes > self.max_resource_bytes {
            return Err(TextTicketError::ResourceTooLarge);
        }
        let revision =
            content_revision.unwrap_or_else(|| format!("{:x}", Sha256::digest(content.as_bytes())));
        if revision.len() != 64 {
            return Err(TextTicketError::InvalidRequest);
        }
        let metadata = TextResourceMetadata {
            encoding: TextEncoding::Utf8,
            size_bytes,
            content_bytes: size_bytes,
            content_revision: revision,
            first_offset: 0,
        };
        let expires_at = now
            .checked_add(self.ttl_millis)
            .ok_or(TextTicketError::InvalidRequest)?;
        self.entries.insert(
            ticket_id.clone(),
            TicketEntry {
                expires_at,
                resource: TicketResource::Memory {
                    content,
                    metadata: metadata.clone(),
                },
            },
        );
        Ok(TextResourceTicket {
            ticket_id,
            expires_at,
            metadata,
        })
    }

    /// # Errors
    ///
    /// 票据不存在、过期、参数无效或源内容变化时返回错误；变化后票据立即删除。
    pub fn read(
        &mut self,
        ticket_id: &str,
        offset: u64,
        max_bytes: usize,
        now: i64,
    ) -> Result<TextResourceChunk, TextTicketError> {
        let Some(entry) = self.entries.get_mut(ticket_id) else {
            return Err(TextTicketError::NotFound);
        };
        if now < 0 || now >= entry.expires_at {
            self.entries.remove(ticket_id);
            return Err(TextTicketError::Expired);
        }
        match entry.resource.read_chunk(offset, max_bytes) {
            Ok(chunk) => Ok(chunk),
            Err(error) => {
                self.entries.remove(ticket_id);
                Err(error)
            }
        }
    }

    /// 在有界大小内读取完整文本，并在完成后再次校验源资源。
    ///
    /// # Errors
    ///
    /// 票据无效、过期、revision 不匹配、资源超限或读取期间变化时返回错误。
    pub fn read_all_verified(
        &mut self,
        ticket_id: &str,
        expected_revision: &str,
        max_bytes: u64,
        now: i64,
    ) -> Result<String, TextTicketError> {
        let Some(entry) = self.entries.get_mut(ticket_id) else {
            return Err(TextTicketError::NotFound);
        };
        if now < 0 || now >= entry.expires_at {
            self.entries.remove(ticket_id);
            return Err(TextTicketError::Expired);
        }
        let metadata = entry.resource.metadata().clone();
        if expected_revision.is_empty() || metadata.content_revision != expected_revision {
            return Err(TextTicketError::InvalidRequest);
        }
        if metadata.content_bytes > max_bytes {
            return Err(TextTicketError::ResourceTooLarge);
        }
        let result = (|| {
            let capacity = usize::try_from(metadata.content_bytes)
                .map_err(|_| TextTicketError::InvalidRequest)?;
            let mut content = String::with_capacity(capacity);
            let mut offset = metadata.first_offset;
            loop {
                let chunk = entry
                    .resource
                    .read_chunk(offset, contexthub_workspace_fs::MAX_TEXT_CHUNK_BYTES)?;
                content.push_str(&chunk.text);
                let Some(next_offset) = chunk.next_offset else {
                    break;
                };
                offset = next_offset;
            }
            entry.resource.verify_unchanged()?;
            Ok(content)
        })();
        if result.is_err() {
            self.entries.remove(ticket_id);
        }
        result
    }

    /// # Errors
    ///
    /// 票据不存在、过期、revision 不匹配或源资源变化时返回错误。
    pub fn verify(
        &mut self,
        ticket_id: &str,
        expected_revision: &str,
        now: i64,
    ) -> Result<(), TextTicketError> {
        let Some(entry) = self.entries.get_mut(ticket_id) else {
            return Err(TextTicketError::NotFound);
        };
        if now < 0 || now >= entry.expires_at {
            self.entries.remove(ticket_id);
            return Err(TextTicketError::Expired);
        }
        if expected_revision.is_empty()
            || entry.resource.metadata().content_revision != expected_revision
        {
            return Err(TextTicketError::InvalidRequest);
        }
        if let Err(error) = entry.resource.verify_unchanged() {
            self.entries.remove(ticket_id);
            return Err(error);
        }
        Ok(())
    }

    fn evict_expired(&mut self, now: i64) {
        self.entries.retain(|_, entry| entry.expires_at > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn tickets_are_bounded_expire_and_invalidate_when_source_changes() {
        let root = test_root();
        fs::write(root.join("note.txt"), "hello world").unwrap();
        let expected = ExpectedTextResourceRevision {
            size_bytes: Some(11),
            checksum_algorithm: None,
            checksum_value: None,
        };
        let mut tickets = TextResourceTickets::new(1, 10, 100);
        let ticket = tickets
            .issue("t1".to_owned(), &root, "note.txt", &expected, 1)
            .unwrap();
        assert_eq!(tickets.read("t1", 0, 256, 2).unwrap().text, "hello world");
        assert!(matches!(
            tickets.issue("t2".to_owned(), &root, "note.txt", &expected, 2),
            Err(TextTicketError::CapacityReached)
        ));
        assert!(matches!(
            tickets.read("t1", 0, 256, ticket.expires_at),
            Err(TextTicketError::Expired)
        ));
        let ticket = tickets
            .issue("t2".to_owned(), &root, "note.txt", &expected, 12)
            .unwrap();
        fs::write(root.join("note.txt"), "changed now").unwrap();
        assert!(matches!(
            tickets.read(&ticket.ticket_id, 0, 256, 13),
            Err(TextTicketError::Source(_))
        ));
        assert!(matches!(
            tickets.read(&ticket.ticket_id, 0, 256, 13),
            Err(TextTicketError::NotFound)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    fn test_root() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ch-ticket-{}-{nanos}", std::process::id()));
        fs::create_dir(&root).unwrap();
        root
    }
}
