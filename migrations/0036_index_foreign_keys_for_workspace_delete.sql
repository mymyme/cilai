-- 删除工作区时按行删除 local_files 与 local_scan_snapshots，SQLite 每删一行都要按外键
-- 查一遍引用它的子表。下列引用字段此前没有可用的索引（已有索引以 workspace_id 开头，
-- 按单字段查找用不上），每次查找都退化为整表扫描：实测删一个一万三千文件的工作区
-- 要 54 秒，期间占着服务主锁。补齐后同一删除降到 0.3 秒。
CREATE INDEX idx_local_files_parent ON local_files(parent_local_file_id);
CREATE INDEX idx_local_files_first_seen_snapshot ON local_files(first_seen_snapshot_id);
CREATE INDEX idx_local_files_last_seen_snapshot ON local_files(last_seen_snapshot_id);
CREATE INDEX idx_local_files_missing_since_snapshot ON local_files(missing_since_snapshot_id);
CREATE INDEX idx_checksum_cache_local_file ON checksum_cache(local_file_id);
CREATE INDEX idx_conflicts_local_file ON conflicts(local_file_id);
CREATE INDEX idx_file_confirmation_text_snapshots_local_file ON file_confirmation_text_snapshots(local_file_id);
CREATE INDEX idx_local_file_observations_local_file ON local_file_observations(local_file_id);
CREATE INDEX idx_sync_plan_items_local_file ON sync_plan_items(local_file_id);
