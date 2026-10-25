use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rusqlite::{Connection, params};

use crate::{PersistenceError, configuration_error, database_error};

/// 正文索引的明确分类；不支持的内容不写入可搜索文本。
#[derive(Debug)]
pub enum FileSearchContent {
    Text(String),
    Binary,
    UnsupportedEncoding,
    TooLarge,
}

impl FileSearchContent {
    const fn status(&self) -> &'static str {
        match self {
            Self::Text(_) => "indexed",
            Self::Binary => "binary",
            Self::UnsupportedEncoding => "unsupported_encoding",
            Self::TooLarge => "too_large",
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            _ => 0,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct SearchIndexState {
    known: std::collections::HashSet<String>,
    pending: HashMap<String, FileSearchContent>,
    pending_bytes: usize,
    active_scans: Arc<AtomicUsize>,
}

impl SearchIndexState {
    pub(super) fn open(connection: &mut Connection) -> Result<Self, PersistenceError> {
        Self::default().collect_unused(connection)?;
        let known = connection
            .prepare("SELECT checksum_value FROM file_search_documents")
            .map_err(database_error("prepare file search versions"))?
            .query_map([], |row| row.get(0))
            .map_err(database_error("query file search versions"))?
            .collect::<Result<_, _>>()
            .map_err(database_error("read file search versions"))?;
        Ok(Self {
            known,
            ..Self::default()
        })
    }

    fn collect_unused(&mut self, connection: &mut Connection) -> Result<(), PersistenceError> {
        if self.active_scans.load(Ordering::Acquire) != 0 {
            return Ok(());
        }
        self.flush(connection)?;
        let transaction = connection
            .transaction()
            .map_err(database_error("start search version cleanup"))?;
        let unused: Vec<(i64, String)> = transaction.prepare(
            "SELECT document_id, checksum_value FROM file_search_documents d
             WHERE NOT EXISTS (SELECT 1 FROM local_files f WHERE f.local_checksum_value=d.checksum_value
               AND f.local_checksum_algorithm='sha256' AND f.materialization_state<>'missing'
               AND f.relative_path NOT GLOB '.*' AND f.relative_path NOT GLOB '*/.*')"
        ).map_err(database_error("prepare unused search versions"))?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(database_error("query unused search versions"))?
            .collect::<Result<_, _>>().map_err(database_error("read unused search versions"))?;
        for (id, _) in &unused {
            transaction
                .execute("DELETE FROM file_search_text WHERE rowid=?1", [id])
                .map_err(database_error("remove unused search text"))?;
            transaction
                .execute(
                    "DELETE FROM file_search_documents WHERE document_id=?1",
                    [id],
                )
                .map_err(database_error("remove unused search version"))?;
        }
        transaction
            .commit()
            .map_err(database_error("commit search version cleanup"))?;
        for (_, checksum) in unused {
            self.known.remove(&checksum);
        }
        Ok(())
    }

    pub(super) fn flush(&mut self, connection: &mut Connection) -> Result<(), PersistenceError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let transaction = connection
            .transaction()
            .map_err(database_error("start file search indexing"))?;
        for (checksum, content) in &self.pending {
            transaction.execute("INSERT INTO file_search_documents(checksum_value,status) VALUES (?1,?2) ON CONFLICT(checksum_value) DO NOTHING", params![checksum, content.status()])
                .map_err(database_error("save file search version"))?;
            let id: i64 = transaction
                .query_row(
                    "SELECT document_id FROM file_search_documents WHERE checksum_value=?1",
                    [checksum],
                    |row| row.get(0),
                )
                .map_err(database_error("read file search document id"))?;
            if let FileSearchContent::Text(text) = content {
                transaction
                    .execute(
                        "INSERT OR REPLACE INTO file_search_text(rowid,content) VALUES (?1,?2)",
                        params![id, text],
                    )
                    .map_err(database_error("index file search text"))?;
            }
        }
        transaction
            .commit()
            .map_err(database_error("commit file search indexing"))?;
        self.known.extend(self.pending.drain().map(|(key, _)| key));
        self.pending_bytes = 0;
        Ok(())
    }
}

/// 从正文观察到最后一批文件事实提交期间，保护尚未关联到文件的内容版本。
pub struct FileSearchScan {
    index: Arc<super::FileContentIndex>,
    active_scans: Arc<AtomicUsize>,
}

impl Drop for FileSearchScan {
    fn drop(&mut self) {
        self.active_scans.fetch_sub(1, Ordering::Release);
    }
}

impl FileSearchScan {
    /// 在当前扫描的保护范围内取得正文索引。
    ///
    /// # Errors
    /// 内容读取或索引写入失败时返回错误。
    pub fn index_content<E: From<PersistenceError>>(
        &self,
        checksum: &str,
        read: impl FnOnce() -> Result<FileSearchContent, E>,
    ) -> Result<(), E> {
        self.index.index_search_content(checksum, read)
    }
}

impl super::FileContentIndex {
    /// 开始一次正文观察；先回收已结束扫描遗留的无引用版本。
    ///
    /// # Errors
    /// 索引锁或清理失败时返回错误。
    pub fn begin_search_scan(self: &Arc<Self>) -> Result<Arc<FileSearchScan>, PersistenceError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| configuration_error("content index lock failed"))?;
        let super::file_content_index::IndexState {
            connection, search, ..
        } = &mut *state;
        search.collect_unused(connection)?;
        search.active_scans.fetch_add(1, Ordering::Release);
        Ok(Arc::new(FileSearchScan {
            index: Arc::clone(self),
            active_scans: Arc::clone(&search.active_scans),
        }))
    }

    /// 回收没有当前文件引用且没有在途扫描保护的正文，同时更新内存版本集合。
    ///
    /// # Errors
    /// 锁或数据库写入失败时返回错误。
    pub fn collect_unused_search_content(&self) -> Result<(), PersistenceError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| configuration_error("content index lock failed"))?;
        let super::file_content_index::IndexState {
            connection, search, ..
        } = &mut *state;
        search.collect_unused(connection)
    }
    /// 是否已按内容版本完成正文分类或索引。
    ///
    /// # Errors
    /// 索引锁异常时返回错误。
    pub fn has_search_content(&self, checksum: &str) -> Result<bool, PersistenceError> {
        let state = self
            .state
            .lock()
            .map_err(|_| configuration_error("content index lock failed"))?;
        Ok(state.search.known.contains(checksum) || state.search.pending.contains_key(checksum))
    }

    /// 在业务服务锁外读取正文；同版本跨工作区复用，待写正文有固定内存预算。
    ///
    /// # Errors
    /// 内容读取、锁或写索引失败时返回错误，不将失败标记为已索引。
    fn index_search_content<E: From<PersistenceError>>(
        &self,
        checksum: &str,
        read: impl FnOnce() -> Result<FileSearchContent, E>,
    ) -> Result<(), E> {
        if self.has_search_content(checksum)? {
            return Ok(());
        }
        let content = read()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| configuration_error("content index lock failed"))?;
        if state.search.known.contains(checksum) || state.search.pending.contains_key(checksum) {
            return Ok(());
        }
        state.search.pending_bytes += content.bytes();
        state.search.pending.insert(checksum.to_owned(), content);
        if state.search.pending_bytes >= 4 * 1024 * 1024 || state.search.pending.len() >= 256 {
            let super::file_content_index::IndexState {
                connection, search, ..
            } = &mut *state;
            search.flush(connection)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_versions_survive_concurrent_scans_and_are_recomputed_after_collection() {
        let root = std::env::temp_dir().join(format!("cilai-search-gc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let mut persistence =
            crate::SqlitePersistence::open(&root.join("state.db"), "0.1.0").unwrap();
        let index = persistence.file_content_index().unwrap();
        let checksum = "a".repeat(64);
        let first = index.begin_search_scan().unwrap();
        let second = index.begin_search_scan().unwrap();
        first
            .index_content::<PersistenceError>(&checksum, || {
                Ok(FileSearchContent::Text("完整正文".into()))
            })
            .unwrap();
        index.flush().unwrap();
        index.collect_unused_search_content().unwrap();
        assert!(index.has_search_content(&checksum).unwrap());
        drop(first);
        index.collect_unused_search_content().unwrap();
        assert!(index.has_search_content(&checksum).unwrap());
        drop(second);
        index.collect_unused_search_content().unwrap();
        assert!(!index.has_search_content(&checksum).unwrap());
        let scan = index.begin_search_scan().unwrap();
        let mut reread = false;
        scan.index_content::<PersistenceError>(&checksum, || {
            reread = true;
            Ok(FileSearchContent::Text("完整正文".into()))
        })
        .unwrap();
        assert!(reread);
        index.flush().unwrap();
        drop(scan);
        drop(index);
        drop(persistence);
        let mut reopened = crate::SqlitePersistence::open(&root.join("state.db"), "0.1.0").unwrap();
        assert!(
            !reopened
                .file_content_index()
                .unwrap()
                .has_search_content(&checksum)
                .unwrap()
        );
        let connection = rusqlite::Connection::open(root.join("state.db")).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM file_search_text", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(connection);
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
