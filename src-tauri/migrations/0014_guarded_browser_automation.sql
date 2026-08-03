PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS browser_automation_sessions (
    session_id TEXT PRIMARY KEY CHECK (session_id GLOB 'browser_*'),
    task_run_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    canonical_origin TEXT NOT NULL,
    destination_binding TEXT NOT NULL,
    document_generation INTEGER NOT NULL DEFAULT 0 CHECK (document_generation >= 0),
    state TEXT NOT NULL CHECK (state IN ('automating','paused','takeover','stopped','closed')),
    current_step TEXT NOT NULL DEFAULT '',
    last_snapshot_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS browser_automation_actions (
    action_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    action_kind TEXT NOT NULL,
    reference_id TEXT,
    destination_origin TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('previewed','approved','executed','blocked','observed','verified','failed')),
    evidence_json TEXT NOT NULL CHECK (json_valid(evidence_json)),
    screenshot_path TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES browser_automation_sessions(session_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS browser_download_quarantine (
    download_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    source_origin TEXT NOT NULL,
    private_path TEXT NOT NULL,
    file_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    byte_count INTEGER NOT NULL CHECK (byte_count >= 0),
    sha256 TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('quarantined','rejected','exported','failed')),
    exported_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES browser_automation_sessions(session_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_browser_sessions_task ON browser_automation_sessions(task_run_id, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_browser_actions_session ON browser_automation_actions(session_id, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_browser_downloads_session ON browser_download_quarantine(session_id, created_at_ms DESC);
