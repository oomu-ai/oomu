PRAGMA foreign_keys = ON;

CREATE TABLE delegation_plans_v2 (
    plan_id TEXT PRIMARY KEY CHECK (plan_id GLOB 'delegation_*'),
    project_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    parent_session_id TEXT,
    parent_model_route TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('planned','running','paused','completed','partial','failed','cancelled')),
    required_child_count INTEGER NOT NULL CHECK (required_child_count BETWEEN 1 AND 8),
    aggregate_budget_json TEXT NOT NULL CHECK (json_valid(aggregate_budget_json)),
    synthesis_json TEXT CHECK (synthesis_json IS NULL OR json_valid(synthesis_json)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

INSERT INTO delegation_plans_v2 SELECT * FROM delegation_plans;

CREATE TABLE delegation_child_runs_v2 (
    child_run_id TEXT PRIMARY KEY CHECK (child_run_id GLOB 'childrun_*'),
    plan_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 7),
    goal TEXT NOT NULL,
    source_scope_json TEXT NOT NULL CHECK (json_valid(source_scope_json)),
    tool_scope_json TEXT NOT NULL CHECK (json_valid(tool_scope_json)),
    model_route TEXT NOT NULL,
    budget_json TEXT NOT NULL CHECK (json_valid(budget_json)),
    state TEXT NOT NULL CHECK (state IN ('planned','running','paused','completed','failed','cancelled','incomplete')),
    progress_summary TEXT NOT NULL DEFAULT '',
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    error_code TEXT,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt BETWEEN 1 AND 4),
    started_at_ms INTEGER,
    completed_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (plan_id, ordinal),
    FOREIGN KEY (plan_id) REFERENCES delegation_plans_v2(plan_id) ON DELETE CASCADE
);

INSERT INTO delegation_child_runs_v2 SELECT * FROM delegation_child_runs;
DROP TABLE delegation_child_runs;
DROP TABLE delegation_plans;
ALTER TABLE delegation_plans_v2 RENAME TO delegation_plans;
ALTER TABLE delegation_child_runs_v2 RENAME TO delegation_child_runs;

CREATE INDEX idx_delegation_plans_task ON delegation_plans(task_run_id, updated_at_ms DESC);
CREATE INDEX idx_delegation_children_plan ON delegation_child_runs(plan_id, ordinal);

CREATE TABLE IF NOT EXISTS work_graph_suggestions (
    suggestion_id TEXT PRIMARY KEY CHECK (suggestion_id GLOB 'suggestion_*'),
    plan_id TEXT NOT NULL,
    child_run_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('research_note','file_patch','workbook_revision','presentation_revision','desktop_action','connector_draft')),
    base_revision TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    summary TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    state TEXT NOT NULL CHECK (state IN ('awaiting_review','accepted','rejected','conflict')),
    rejection_reason TEXT,
    created_at_ms INTEGER NOT NULL,
    reviewed_at_ms INTEGER,
    FOREIGN KEY (plan_id) REFERENCES delegation_plans(plan_id) ON DELETE CASCADE,
    FOREIGN KEY (child_run_id) REFERENCES delegation_child_runs(child_run_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS analysis_runs (
    analysis_id TEXT PRIMARY KEY CHECK (analysis_id GLOB 'analysis_*'),
    project_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('completed','failed')),
    answer TEXT NOT NULL,
    table_json TEXT NOT NULL CHECK (json_valid(table_json)),
    chart_json TEXT NOT NULL CHECK (json_valid(chart_json)),
    method_json TEXT NOT NULL CHECK (json_valid(method_json)),
    input_sha256 TEXT NOT NULL CHECK (length(input_sha256)=64),
    output_sha256 TEXT NOT NULL CHECK (length(output_sha256)=64),
    environment_sha256 TEXT NOT NULL CHECK (length(environment_sha256)=64),
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE,
    FOREIGN KEY (source_id) REFERENCES project_sources(source_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_work_graph_suggestions_plan ON work_graph_suggestions(plan_id, state, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_analysis_runs_task ON analysis_runs(task_run_id, completed_at_ms DESC);
