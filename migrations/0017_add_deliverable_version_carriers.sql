CREATE TABLE deliverable_version_carriers (
    carrier_id TEXT PRIMARY KEY CHECK (length(carrier_id) > 0),
    version_id TEXT NOT NULL REFERENCES deliverable_versions(version_id) ON DELETE RESTRICT,
    carrier_type TEXT NOT NULL CHECK (carrier_type IN ('directory', 'commit', 'external', 'project_version')),
    carrier_role TEXT NOT NULL CHECK (carrier_role IN ('primary', 'supporting')),
    label TEXT NOT NULL CHECK (length(label) > 0),
    reference TEXT NOT NULL CHECK (length(reference) > 0),
    version_label TEXT NOT NULL CHECK (length(version_label) > 0),
    source_project_id TEXT REFERENCES projects(project_id) ON DELETE RESTRICT,
    source_deliverable_id TEXT REFERENCES deliverables(deliverable_id) ON DELETE RESTRICT,
    source_version_id TEXT REFERENCES deliverable_versions(version_id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK (
        (carrier_type = 'project_version' AND source_project_id IS NOT NULL AND source_deliverable_id IS NOT NULL AND source_version_id IS NOT NULL)
        OR
        (carrier_type <> 'project_version' AND source_project_id IS NULL AND source_deliverable_id IS NULL AND source_version_id IS NULL)
    )
) STRICT;

CREATE INDEX idx_deliverable_version_carriers_version
    ON deliverable_version_carriers(version_id, carrier_role, carrier_type, carrier_id);

CREATE INDEX idx_deliverable_version_carriers_source
    ON deliverable_version_carriers(source_version_id, version_id)
    WHERE source_version_id IS NOT NULL;
