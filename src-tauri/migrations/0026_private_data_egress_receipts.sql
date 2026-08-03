CREATE TABLE IF NOT EXISTS private_data_egress_receipts (
    receipt_id TEXT PRIMARY KEY,
    source_digest TEXT NOT NULL,
    destination_provider_id TEXT NOT NULL,
    destination_model_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    allowed_representation TEXT NOT NULL CHECK (
        allowed_representation IN ('local_summary', 'redacted_excerpt', 'full_result')
    ),
    representation_digest TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    consumed_at_ms INTEGER,
    signature_json TEXT NOT NULL,
    dispatch_id TEXT NOT NULL UNIQUE,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_private_egress_receipts_dispatch
    ON private_data_egress_receipts(dispatch_id, consumed_at_ms);
