ALTER TABLE sync_plans ADD COLUMN client_operation_id TEXT;
ALTER TABLE sync_plans ADD COLUMN request_fingerprint TEXT;
ALTER TABLE sync_plan_items ADD COLUMN frozen_local_change_state TEXT;
ALTER TABLE sync_plan_items ADD COLUMN frozen_confirmation_state TEXT;
ALTER TABLE sync_plan_items ADD COLUMN frozen_conflict_state TEXT;
ALTER TABLE sync_executions ADD COLUMN client_operation_id TEXT;
ALTER TABLE sync_executions ADD COLUMN request_fingerprint TEXT;
ALTER TABLE conflict_resolutions ADD COLUMN client_operation_id TEXT;

CREATE UNIQUE INDEX idx_sync_plans_client_operation
    ON sync_plans(client_operation_id) WHERE client_operation_id IS NOT NULL;
CREATE UNIQUE INDEX idx_sync_executions_client_operation
    ON sync_executions(client_operation_id) WHERE client_operation_id IS NOT NULL;
CREATE UNIQUE INDEX idx_conflict_resolutions_client_operation
    ON conflict_resolutions(client_operation_id) WHERE client_operation_id IS NOT NULL;
