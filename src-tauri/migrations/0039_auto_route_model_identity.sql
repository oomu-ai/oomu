ALTER TABLE active_session_configs
ADD COLUMN local_model_source TEXT NOT NULL DEFAULT 'legacy_unverified'
CHECK(local_model_source IN (
    'legacy_unverified',
    'explicit_session',
    'agent_assignment',
    'startup_default',
    'verified_legacy_repair',
    'needs_user_choice'
));

ALTER TABLE active_session_configs
ADD COLUMN local_model_reconciled_at_ms INTEGER;

CREATE TABLE IF NOT EXISTS auto_route_baseline_backups (
    session_id TEXT PRIMARY KEY,
    provider_id TEXT,
    model_id TEXT,
    reasoning_depth TEXT NOT NULL,
    context_budget INTEGER NOT NULL,
    local_model_source TEXT NOT NULL,
    backed_up_at_ms INTEGER NOT NULL,
    FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
);
