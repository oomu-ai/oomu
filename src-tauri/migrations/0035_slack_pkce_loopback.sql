DROP INDEX IF EXISTS idx_oauth_attempts_expiry;

ALTER TABLE connector_oauth_attempts RENAME TO connector_oauth_attempts_legacy;

CREATE TABLE connector_oauth_attempts (
    attempt_id TEXT PRIMARY KEY,
    connector_id TEXT NOT NULL,
    state_hash TEXT NOT NULL,
    redirect_uri TEXT NOT NULL CHECK (
        redirect_uri LIKE 'http://127.0.0.1:%/oauth/callback'
        OR redirect_uri LIKE 'http://localhost:%/oauth/callback'
    ),
    expires_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    outcome TEXT NOT NULL DEFAULT 'pending' CHECK (outcome IN ('pending','completed','rejected','expired','failed')),
    created_at_ms INTEGER NOT NULL
);

INSERT INTO connector_oauth_attempts (
    attempt_id, connector_id, state_hash, redirect_uri,
    expires_at_ms, completed_at_ms, outcome, created_at_ms
)
SELECT
    attempt_id, connector_id, state_hash, redirect_uri,
    expires_at_ms, completed_at_ms, outcome, created_at_ms
FROM connector_oauth_attempts_legacy;

DROP TABLE connector_oauth_attempts_legacy;

CREATE INDEX idx_oauth_attempts_expiry
ON connector_oauth_attempts(outcome, expires_at_ms);
