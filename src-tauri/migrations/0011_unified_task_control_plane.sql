PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS task_runs (
    task_run_id TEXT PRIMARY KEY CHECK (task_run_id GLOB 'taskrun_*'),
    task_id TEXT NOT NULL CHECK (task_id GLOB 'task_*'),
    project_id TEXT,
    runtime_kind TEXT NOT NULL CHECK (runtime_kind IN ('taskflow', 'workflow', 'agent', 'queued_message')),
    runtime_record_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'planning', 'awaiting_approval', 'running', 'blocked', 'completed', 'failed', 'cancelled')),
    origin TEXT NOT NULL,
    correlation_id TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    last_error TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    acknowledged_at_ms INTEGER,
    recovery_state TEXT NOT NULL DEFAULT 'not_required'
        CHECK (recovery_state IN ('not_required', 'reconciled', 'recoverable', 'lost', 'runtime_unavailable')),
    UNIQUE(runtime_kind, runtime_record_id),
    FOREIGN KEY(project_id) REFERENCES projects(project_id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS task_events (
    task_run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    event_json TEXT NOT NULL CHECK (json_valid(event_json)),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(task_run_id, sequence),
    FOREIGN KEY(task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS task_effects (
    task_run_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    effect_kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('reserved', 'executed', 'verified')),
    result_digest TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(task_run_id, idempotency_key),
    FOREIGN KEY(task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS task_recovery_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_run_id TEXT NOT NULL,
    previous_state TEXT NOT NULL,
    resolved_state TEXT NOT NULL,
    decision TEXT NOT NULL,
    next_action TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY(task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_task_runs_project_state ON task_runs(project_id, state, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_task_runs_runtime ON task_runs(runtime_kind, runtime_record_id);
CREATE INDEX IF NOT EXISTS idx_task_events_reconnect ON task_events(task_run_id, sequence);
CREATE INDEX IF NOT EXISTS idx_task_runs_terminal_retention ON task_runs(state, completed_at_ms);
