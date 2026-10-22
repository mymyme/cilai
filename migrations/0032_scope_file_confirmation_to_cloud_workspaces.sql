ALTER TABLE file_confirmations ADD COLUMN workspace_kind_at_confirmation TEXT NOT NULL DEFAULT 'local'
    CHECK (workspace_kind_at_confirmation IN ('local', 'cloud_team'));

UPDATE file_confirmations
SET workspace_kind_at_confirmation = 'cloud_team'
WHERE EXISTS (
    SELECT 1 FROM workspaces w
    JOIN cloud_workspace_operations operation ON operation.workspace_id = w.workspace_id
    WHERE w.workspace_id = file_confirmations.workspace_id
      AND w.workspace_kind = 'cloud_team'
      AND operation.state = 'bound'
      AND file_confirmations.created_at > operation.updated_at
);

UPDATE file_states
SET confirmation_state = 'not_required',
    baseline_relation = 'platform_absent',
    conflict_state = CASE WHEN conflict_state = 'metadata_conflict' THEN conflict_state ELSE 'none' END,
    display_state = CASE
        WHEN conflict_state = 'metadata_conflict' THEN 'metadata_error'
        WHEN EXISTS (
            SELECT 1 FROM local_files lf
            WHERE lf.local_file_id = file_states.local_file_id AND lf.materialization_state = 'missing'
        ) THEN 'local_missing'
        WHEN EXISTS (
            SELECT 1 FROM local_files lf
            WHERE lf.local_file_id = file_states.local_file_id
              AND (lf.scan_state IN ('pending', 'scanning') OR lf.materialization_state = 'materializing')
        ) THEN 'scanning'
        WHEN EXISTS (
            SELECT 1 FROM local_files lf
            WHERE lf.local_file_id = file_states.local_file_id
              AND lf.materialization_state = 'not_materialized'
        ) THEN 'not_materialized'
        WHEN EXISTS (
            SELECT 1 FROM local_files lf
            WHERE lf.local_file_id = file_states.local_file_id
              AND (lf.scan_state = 'unreadable' OR lf.materialization_state IN ('unsupported', 'invalid'))
        ) THEN 'unavailable'
        ELSE 'local_ready'
    END,
    capability_bits = capability_bits & ~246,
    state_revision = state_revision + 1
WHERE local_file_id IN (
    SELECT lf.local_file_id FROM local_files lf
    JOIN workspaces w ON w.workspace_id = lf.workspace_id
    WHERE w.workspace_kind = 'local'
);
