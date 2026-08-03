CREATE TABLE IF NOT EXISTS private_egress_confirmation_challenges (
    challenge_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    generation_token TEXT NOT NULL,
    destination_provider_id TEXT NOT NULL,
    destination_model_id TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    allowed_representation TEXT NOT NULL CHECK (
        allowed_representation IN ('local_summary', 'redacted_excerpt', 'full_result')
    ),
    representation_digest TEXT NOT NULL,
    source_names_json TEXT NOT NULL,
    decision TEXT NOT NULL DEFAULT 'pending' CHECK (
        decision IN ('pending', 'approved', 'denied', 'consumed')
    ),
    expires_at_ms INTEGER NOT NULL,
    decided_at_ms INTEGER,
    consumed_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, turn_id, generation_token)
);

CREATE INDEX IF NOT EXISTS idx_private_egress_confirmation_pending
    ON private_egress_confirmation_challenges(
        session_id, turn_id, generation_token, decision, expires_at_ms
    );
