ALTER TABLE accounts ADD COLUMN identity_kind TEXT NOT NULL DEFAULT 'platform'
    CHECK (identity_kind IN ('platform', 'local'));

INSERT INTO accounts(
    account_id,
    display_name,
    account_status,
    created_at,
    updated_at,
    identity_kind
) VALUES (
    'local-profile:default',
    '本机操作人',
    'active',
    0,
    0,
    'local'
);

ALTER TABLE file_confirmations ADD COLUMN client_operation_id TEXT;
ALTER TABLE file_confirmations ADD COLUMN request_fingerprint TEXT;
ALTER TABLE file_confirmations ADD COLUMN confirmed_file_revision INTEGER
    CHECK (confirmed_file_revision >= 0);

CREATE UNIQUE INDEX idx_file_confirmations_client_operation
    ON file_confirmations(client_operation_id)
    WHERE client_operation_id IS NOT NULL;

