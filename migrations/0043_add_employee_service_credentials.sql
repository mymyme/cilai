CREATE TABLE employee_service_credentials (
    tenant_id TEXT NOT NULL,
    membership_id TEXT NOT NULL,
    capability_type TEXT NOT NULL,
    api_key TEXT NOT NULL,
    key_prefix TEXT NOT NULL,
    gateway_url TEXT NOT NULL,
    issued_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (tenant_id, membership_id, capability_type),
    CHECK (length(tenant_id) > 0),
    CHECK (length(membership_id) > 0),
    CHECK (length(capability_type) > 0),
    CHECK (length(api_key) > 0),
    CHECK (length(key_prefix) > 0),
    CHECK (length(gateway_url) > 0)
) STRICT;
