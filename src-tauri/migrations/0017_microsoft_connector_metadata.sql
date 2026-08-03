PRAGMA foreign_keys = ON;

-- Non-secret identity evidence for account/tenant clarity in the connector UI.
-- OAuth tokens and refresh tokens remain exclusively in the OS credential store.
CREATE TABLE IF NOT EXISTS connector_account_metadata (
    connector_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL CHECK(length(trim(tenant_id)) > 0),
    tenant_label TEXT NOT NULL DEFAULT '',
    account_id TEXT NOT NULL CHECK(length(trim(account_id)) > 0),
    account_principal TEXT NOT NULL DEFAULT '',
    account_kind TEXT NOT NULL CHECK(account_kind IN ('personal','work')),
    identity_binding_hash TEXT NOT NULL CHECK(length(identity_binding_hash) = 64),
    data_routing_json TEXT NOT NULL CHECK(json_valid(data_routing_json)),
    consent_reviewed_at_ms INTEGER NOT NULL,
    identity_verified_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (connector_id) REFERENCES connector_accounts(connector_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_connector_account_metadata_tenant
    ON connector_account_metadata(tenant_id, account_kind, updated_at_ms);
