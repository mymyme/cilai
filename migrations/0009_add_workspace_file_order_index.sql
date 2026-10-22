CREATE INDEX idx_local_files_workspace_order
ON local_files(workspace_id, normalized_relative_path, local_file_id);
