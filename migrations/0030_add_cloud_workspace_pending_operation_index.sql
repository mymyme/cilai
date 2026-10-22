CREATE UNIQUE INDEX idx_cloud_workspace_one_pending_per_project
    ON cloud_workspace_operations(project_id)
    WHERE state IN ('prepared','creating_remote','pulling_remote','uploading_initial');
