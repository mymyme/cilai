ALTER TABLE confirmation_invalidations RENAME TO confirmation_invalidations_v5;

CREATE TABLE confirmation_invalidations (
    invalidation_id TEXT PRIMARY KEY CHECK (length(invalidation_id) > 0),
    confirmation_id TEXT NOT NULL REFERENCES file_confirmations(confirmation_id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (reason IN ('local_content_changed', 'identity_changed', 'baseline_changed', 'permission_changed', 'conflict_detected')),
    previous_checksum TEXT,
    current_checksum TEXT,
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    correlation_id TEXT
) STRICT;

INSERT INTO confirmation_invalidations(
    invalidation_id,
    confirmation_id,
    reason,
    previous_checksum,
    current_checksum,
    occurred_at,
    correlation_id
)
SELECT
    invalidation_id,
    confirmation_id,
    reason,
    previous_checksum,
    current_checksum,
    occurred_at,
    correlation_id
FROM confirmation_invalidations_v5;

DROP TABLE confirmation_invalidations_v5;

CREATE INDEX idx_confirmation_invalidations_history
    ON confirmation_invalidations(confirmation_id, occurred_at DESC);
