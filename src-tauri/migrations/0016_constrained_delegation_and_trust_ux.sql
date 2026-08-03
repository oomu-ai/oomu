PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS delegation_plans (
    plan_id TEXT PRIMARY KEY CHECK (plan_id GLOB 'delegation_*'),
    project_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    parent_session_id TEXT,
    parent_model_route TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('planned','running','completed','partial','failed','cancelled')),
    required_child_count INTEGER NOT NULL CHECK (required_child_count BETWEEN 1 AND 3),
    aggregate_budget_json TEXT NOT NULL CHECK (json_valid(aggregate_budget_json)),
    synthesis_json TEXT CHECK (synthesis_json IS NULL OR json_valid(synthesis_json)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS delegation_child_runs (
    child_run_id TEXT PRIMARY KEY CHECK (child_run_id GLOB 'childrun_*'),
    plan_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 2),
    goal TEXT NOT NULL,
    source_scope_json TEXT NOT NULL CHECK (json_valid(source_scope_json)),
    tool_scope_json TEXT NOT NULL CHECK (json_valid(tool_scope_json)),
    model_route TEXT NOT NULL,
    budget_json TEXT NOT NULL CHECK (json_valid(budget_json)),
    state TEXT NOT NULL CHECK (state IN ('planned','running','completed','failed','cancelled','incomplete')),
    progress_summary TEXT NOT NULL DEFAULT '',
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    error_code TEXT,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt BETWEEN 1 AND 4),
    started_at_ms INTEGER,
    completed_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (plan_id, ordinal),
    FOREIGN KEY (plan_id) REFERENCES delegation_plans(plan_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS reviewed_approval_scopes (
    grant_id TEXT PRIMARY KEY CHECK (grant_id GLOB 'trustgrant_*'),
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('task','project_path','persistent')),
    principal TEXT NOT NULL,
    project_id TEXT,
    task_run_id TEXT,
    action_class TEXT NOT NULL,
    canonical_resource TEXT NOT NULL,
    argument_class TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    max_uses INTEGER NOT NULL CHECK (max_uses BETWEEN 1 AND 10000),
    used_count INTEGER NOT NULL DEFAULT 0 CHECK (used_count >= 0),
    resource_budget_json TEXT NOT NULL CHECK (json_valid(resource_budget_json)),
    reviewed_at_ms INTEGER NOT NULL,
    revoked_at_ms INTEGER,
    last_used_at_ms INTEGER,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS approval_scope_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    grant_id TEXT,
    task_run_id TEXT,
    event_type TEXT NOT NULL CHECK (event_type IN ('granted','used','denied','expired','revoked','mandatory_reconfirm')),
    action_class TEXT NOT NULL,
    canonical_resource_hash TEXT NOT NULL,
    detail_json TEXT NOT NULL CHECK (json_valid(detail_json)),
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY (grant_id) REFERENCES reviewed_approval_scopes(grant_id) ON DELETE SET NULL,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_delegation_plans_task ON delegation_plans(task_run_id, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_delegation_children_plan ON delegation_child_runs(plan_id, ordinal);
CREATE INDEX IF NOT EXISTS idx_approval_scope_match ON reviewed_approval_scopes(action_class, project_id, task_run_id, expires_at_ms);
CREATE INDEX IF NOT EXISTS idx_approval_scope_audit_task ON approval_scope_audit(task_run_id, created_at_ms DESC);
