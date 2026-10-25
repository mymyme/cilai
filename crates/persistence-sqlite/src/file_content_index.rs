use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use rusqlite::{Connection, OpenFlags, params};

use crate::{
    PersistenceError, SqlitePersistence, configuration_error, configure_connection, database_error,
};

#[derive(Debug)]
pub(super) struct IndexState {
    pub(super) connection: Connection,
    pub(super) search: super::file_search_index::SearchIndexState,
    records: HashMap<String, (String, String)>,
    file_locks: HashMap<String, Weak<Mutex<()>>>,
    pending: HashSet<String>,
}

/// 当前本地 Profile 内按实际文件身份共享的内容摘要。
#[derive(Debug)]
pub struct FileContentIndex {
    pub(super) state: Mutex<IndexState>,
}

impl FileContentIndex {
    fn open(path: &Path) -> Result<Self, PersistenceError> {
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(database_error("open shared content index"))?;
        configure_connection(&connection)?;
        let search = super::file_search_index::SearchIndexState::open(&mut connection)?;
        let records = connection
            .prepare(
                "SELECT file_identity, fingerprint, checksum_value FROM shared_file_content_index",
            )
            .map_err(database_error("prepare shared content index"))?
            .query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))
            .map_err(database_error("query shared content index"))?
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(database_error("read shared content index"))?;
        Ok(Self {
            state: Mutex::new(IndexState {
                connection,
                search,
                records,
                file_locks: HashMap::new(),
                pending: HashSet::new(),
            }),
        })
    }

    /// 查询完全相同文件指纹的摘要。
    ///
    /// # Errors
    /// 内容索引锁异常时返回错误。
    pub fn lookup(
        &self,
        identity: &str,
        fingerprint: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let state = self
            .state
            .lock()
            .map_err(|_| configuration_error("content index lock failed"))?;
        Ok(state
            .records
            .get(identity)
            .filter(|record| record.0 == fingerprint)
            .map(|record| record.1.clone()))
    }

    /// 对相同实际文件的并发内容读取合并一次，成功结果待扫描结束后批量持久化。
    ///
    /// # Errors
    /// 内容读取或锁失败时返回错误，不保存失败结果。
    pub fn get_or_compute<E: From<PersistenceError>>(
        &self,
        identity: &str,
        fingerprint: &str,
        compute: impl FnOnce() -> Result<String, E>,
    ) -> Result<String, E> {
        let file_lock = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| configuration_error("content index lock failed"))?;
            let lock = state
                .file_locks
                .get(identity)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| Arc::new(Mutex::new(())));
            state
                .file_locks
                .insert(identity.to_owned(), Arc::downgrade(&lock));
            lock
        };
        let _file_guard = file_lock
            .lock()
            .map_err(|_| configuration_error("file content lock failed"))?;
        if let Some(checksum) = self.lookup(identity, fingerprint)? {
            return Ok(checksum);
        }
        let checksum = compute()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| configuration_error("content index lock failed"))?;
        state.pending.insert(identity.to_owned());
        state.records.insert(
            identity.to_owned(),
            (fingerprint.to_owned(), checksum.clone()),
        );
        Ok(checksum)
    }

    /// 在提交文件观测之前批量保存摘要，避免每个文件单独提交数据库。
    ///
    /// # Errors
    /// 锁或数据库事务失败时返回错误，保留待保存结果以便显式重试。
    pub fn flush(&self) -> Result<(), PersistenceError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| configuration_error("content index lock failed"))?;
        let IndexState {
            connection, search, ..
        } = &mut *state;
        search.flush(connection)?;
        if state.pending.is_empty() {
            return Ok(());
        }
        let IndexState {
            connection,
            records,
            pending,
            ..
        } = &mut *state;
        let transaction = connection
            .transaction()
            .map_err(database_error("start shared content index transaction"))?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO shared_file_content_index(file_identity,fingerprint,checksum_value) VALUES (?1,?2,?3)
                 ON CONFLICT(file_identity) DO UPDATE SET fingerprint=excluded.fingerprint,checksum_value=excluded.checksum_value"
            ).map_err(database_error("prepare shared content index write"))?;
            for identity in pending.iter() {
                let (fingerprint, checksum) = &records[identity];
                statement
                    .execute(params![identity, fingerprint, checksum])
                    .map_err(database_error("save shared content index"))?;
            }
        }
        transaction
            .commit()
            .map_err(database_error("commit shared content index"))?;
        pending.clear();
        Ok(())
    }
}

impl SqlitePersistence {
    /// 内容索引连接由 Local App Server 持有，不占用业务服务锁读取文件内容。
    ///
    /// # Errors
    /// 数据库打开或索引读取失败时返回错误。
    pub fn file_content_index(&mut self) -> Result<Arc<FileContentIndex>, PersistenceError> {
        if let Some(index) = &self.file_content_index {
            return Ok(Arc::clone(index));
        }
        let index = Arc::new(FileContentIndex::open(&self.database_path)?);
        self.file_content_index = Some(Arc::clone(&index));
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn shared_content_is_computed_once_and_survives_restart() {
        let root = std::env::temp_dir().join(format!(
            "shared-index-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let database = root.join("state.db");
        let mut persistence = SqlitePersistence::open(&database, "0.1.0").unwrap();
        let index = persistence.file_content_index().unwrap();
        let calls = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let index = &index;
                let calls = &calls;
                scope.spawn(move || {
                    let value = index
                        .get_or_compute::<PersistenceError>("file-1", "4:100", || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok("a".repeat(64))
                        })
                        .unwrap();
                    assert_eq!(value, "a".repeat(64));
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(index.lookup("file-1", "4:101").unwrap(), None);
        assert_eq!(index.lookup("file-2", "4:100").unwrap(), None);
        index.flush().unwrap();
        drop(index);
        drop(persistence);
        let mut reopened = SqlitePersistence::open(&database, "0.1.0").unwrap();
        let index = reopened.file_content_index().unwrap();
        assert_eq!(
            index
                .get_or_compute::<PersistenceError>("file-1", "4:100", || panic!(
                    "content must be reused"
                ))
                .unwrap(),
            "a".repeat(64)
        );
        assert!(
            index
                .get_or_compute::<PersistenceError>("file-1", "4:101", || Err(configuration_error(
                    "read failed"
                )))
                .is_err()
        );
        assert_eq!(index.lookup("file-1", "4:101").unwrap(), None);
        assert_eq!(
            index
                .get_or_compute::<PersistenceError>("file-1", "4:101", || Ok("b".repeat(64)))
                .unwrap(),
            "b".repeat(64)
        );
        assert_eq!(index.lookup("file-1", "4:100").unwrap(), None);
        drop(index);
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
