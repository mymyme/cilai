CREATE TEMP TABLE cloud_mirror_migration AS
SELECT w.workspace_id AS local_workspace_id,
       CASE WHEN b.team_space_id IS NOT NULL AND b.binding_state='bound'
         THEN COALESCE((SELECT direct.workspace_id FROM workspaces direct
                        JOIN workspace_platform_bindings db ON db.workspace_id=direct.workspace_id
                        WHERE direct.workspace_kind='cloud_team'
                          AND direct.local_root='cilai-cloud://' || b.team_space_id
                          AND db.account_id=b.account_id AND db.binding_state='bound' LIMIT 1),
                       'cloud-workspace-retired-' || w.workspace_id)
         ELSE NULL END AS cloud_workspace_id,
       w.local_root, b.team_space_id, b.binding_state AS previous_binding_state
FROM workspaces w LEFT JOIN workspace_platform_bindings b ON b.workspace_id=w.workspace_id
WHERE w.workspace_kind='cloud_team' AND w.local_root NOT LIKE 'cilai-cloud://%';

UPDATE workspace_platform_bindings SET binding_state='disconnected'
WHERE workspace_id IN (SELECT local_workspace_id FROM cloud_mirror_migration);

INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at,workspace_kind)
SELECT DISTINCT m.cloud_workspace_id,w.display_name,'cilai-cloud://' || m.team_space_id,'ready',0,w.created_at,w.updated_at,'cloud_team'
FROM cloud_mirror_migration m JOIN workspaces w ON w.workspace_id=m.local_workspace_id
WHERE m.cloud_workspace_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM workspaces target WHERE target.workspace_id=m.cloud_workspace_id);

INSERT INTO workspace_platform_bindings(workspace_id,account_id,organization_id,platform_workspace_id,project_id,platform_root,binding_state,last_platform_check_at,created_at,updated_at,team_space_id,permission_role,permission_revision,access_mode)
SELECT m.cloud_workspace_id,b.account_id,b.organization_id,m.team_space_id,b.project_id,'/','bound',b.last_platform_check_at,b.created_at,b.updated_at,m.team_space_id,b.permission_role,b.permission_revision,'remote_direct'
FROM cloud_mirror_migration m JOIN workspace_platform_bindings b ON b.workspace_id=m.local_workspace_id
WHERE m.cloud_workspace_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM workspace_platform_bindings target WHERE target.workspace_id=m.cloud_workspace_id);

INSERT INTO workspace_exports(workspace_id,desired_revision,exported_revision,export_state,updated_at)
SELECT DISTINCT m.cloud_workspace_id,0,0,'current',w.updated_at
FROM cloud_mirror_migration m JOIN workspaces w ON w.workspace_id=m.local_workspace_id
WHERE m.cloud_workspace_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM workspace_exports target WHERE target.workspace_id=m.cloud_workspace_id);

INSERT INTO project_workspaces(project_id,workspace_id,role,created_at)
SELECT links.project_id,m.cloud_workspace_id,'supporting',links.created_at
FROM cloud_mirror_migration m JOIN project_workspaces links ON links.workspace_id=m.local_workspace_id
WHERE m.cloud_workspace_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM project_workspaces target WHERE target.project_id=links.project_id AND target.workspace_id=m.cloud_workspace_id);

CREATE TABLE cloud_mirror_retirements (
    local_workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE CASCADE,
    cloud_workspace_id TEXT REFERENCES workspaces(workspace_id) ON DELETE SET NULL,
    local_root TEXT NOT NULL,
    team_space_id TEXT,
    previous_binding_state TEXT
) STRICT;
INSERT INTO cloud_mirror_retirements SELECT * FROM cloud_mirror_migration;
CREATE INDEX idx_cloud_mirror_retirements_cloud ON cloud_mirror_retirements(cloud_workspace_id);

UPDATE direct_cloud_workspace_operations
SET workspace_id=(SELECT cloud_workspace_id FROM cloud_mirror_migration WHERE local_workspace_id=workspace_id)
WHERE workspace_id IN (SELECT local_workspace_id FROM cloud_mirror_migration WHERE cloud_workspace_id IS NOT NULL);

UPDATE workspaces SET workspace_kind='local',state_revision=state_revision+1,
    display_name=display_name || '（本地副本）',
    local_status=CASE WHEN local_status IN ('creating','scanning') THEN 'attention_required' ELSE local_status END
WHERE workspace_id IN (SELECT local_workspace_id FROM cloud_mirror_migration);

UPDATE workspace_exports SET desired_revision=desired_revision+1,export_state='pending'
WHERE workspace_id IN (SELECT local_workspace_id FROM cloud_mirror_migration);

UPDATE sync_plans SET state='invalidated',invalidated_reason='cloud_workspace_mirror_retired'
WHERE workspace_id IN (SELECT local_workspace_id FROM cloud_mirror_migration)
  AND state IN ('building','ready','executing');
UPDATE sync_executions SET state='cancelled',error_reason='cloud_workspace_mirror_retired'
WHERE plan_id IN (SELECT plan_id FROM sync_plans WHERE invalidated_reason='cloud_workspace_mirror_retired')
  AND state IN ('queued','running','outcome_unknown');
UPDATE tasks SET state='cancelled',cancellable=0,error_reason='cloud_workspace_mirror_retired'
WHERE task_id IN (SELECT task_id FROM sync_executions WHERE error_reason='cloud_workspace_mirror_retired');
UPDATE cloud_workspace_operations SET state='failed',failure_reason='cloud_workspace_mirror_retired'
WHERE state IN ('prepared','service_unavailable','creating_remote','pulling_remote','uploading_initial','bound');
DROP TABLE cloud_mirror_migration;
