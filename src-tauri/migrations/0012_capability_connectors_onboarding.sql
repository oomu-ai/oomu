PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS connector_accounts (
    connector_id TEXT PRIMARY KEY,
    manifest_id TEXT NOT NULL,
    credential_ref TEXT NOT NULL UNIQUE,
    account_label TEXT NOT NULL DEFAULT '',
    account_subject_hash TEXT,
    granted_scopes_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(granted_scopes_json)),
    token_expires_at_ms INTEGER,
    refresh_expires_at_ms INTEGER,
    connection_state TEXT NOT NULL DEFAULT 'configured' CHECK (connection_state IN ('configured','authorized','reachable','degraded','expired','unsupported','blocked','disconnected')),
    schema_version INTEGER NOT NULL,
    last_probe_at_ms INTEGER,
    last_probe_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS connector_project_bindings (
    connector_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0,1)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (connector_id, project_id),
    FOREIGN KEY (connector_id) REFERENCES connector_accounts(connector_id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS connector_oauth_attempts (
    attempt_id TEXT PRIMARY KEY,
    connector_id TEXT NOT NULL,
    state_hash TEXT NOT NULL,
    redirect_uri TEXT NOT NULL CHECK (redirect_uri LIKE 'http://127.0.0.1:%/oauth/callback'),
    expires_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    outcome TEXT NOT NULL DEFAULT 'pending' CHECK (outcome IN ('pending','completed','rejected','expired','failed')),
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS setup_progress (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    current_step TEXT NOT NULL,
    model_path TEXT,
    completion_channel TEXT,
    sample_project_id TEXT,
    completed_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);

INSERT OR IGNORE INTO setup_progress (singleton, current_step, completed_at_ms, updated_at_ms)
SELECT 1,
       CASE WHEN EXISTS (SELECT 1 FROM chat_sessions LIMIT 1)
                  OR EXISTS (SELECT 1 FROM projects LIMIT 1)
            THEN 'finished' ELSE 'welcome' END,
       CASE WHEN EXISTS (SELECT 1 FROM chat_sessions LIMIT 1)
                  OR EXISTS (SELECT 1 FROM projects LIMIT 1)
            THEN CAST(strftime('%s','now') AS INTEGER) * 1000 ELSE NULL END,
       CAST(strftime('%s','now') AS INTEGER) * 1000;

CREATE TABLE IF NOT EXISTS activation_receipts (
    receipt_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    model_route TEXT NOT NULL,
    capability_snapshot_json TEXT NOT NULL CHECK (json_valid(capability_snapshot_json)),
    verified_at_ms INTEGER NOT NULL,
    UNIQUE(project_id, task_run_id),
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE RESTRICT,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS setup_sample_tasks (
    sample_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('running','completed','failed')),
    output_digest TEXT,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_connector_bindings_project ON connector_project_bindings(project_id, enabled);
CREATE INDEX IF NOT EXISTS idx_connector_accounts_state ON connector_accounts(connection_state, updated_at_ms);
CREATE INDEX IF NOT EXISTS idx_oauth_attempts_expiry ON connector_oauth_attempts(outcome, expires_at_ms);

INSERT OR IGNORE INTO connector_accounts (connector_id,manifest_id,credential_ref,account_label,granted_scopes_json,connection_state,schema_version,created_at_ms,updated_at_ms)
VALUES ('connector_00000000-0000-4000-8000-000000000001','apple_apps','builtin_apple_runtime','This Mac','[]','configured',1,CAST(strftime('%s','now') AS INTEGER)*1000,CAST(strftime('%s','now') AS INTEGER)*1000);
INSERT OR IGNORE INTO connector_accounts (connector_id,manifest_id,credential_ref,account_label,granted_scopes_json,connection_state,schema_version,created_at_ms,updated_at_ms)
VALUES ('connector_00000000-0000-4000-8000-000000000002','mcp_runtime','builtin_mcp_runtime','Configured MCP servers','[]','configured',1,CAST(strftime('%s','now') AS INTEGER)*1000,CAST(strftime('%s','now') AS INTEGER)*1000);
