PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS learning_offers (
    offer_id TEXT PRIMARY KEY CHECK (offer_id GLOB 'learning_*'),
    project_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('procedure','preference','correction','verification_rule','failure_avoidance')),
    status TEXT NOT NULL CHECK (status IN ('proposed','accepted','rejected','postponed')),
    summary TEXT NOT NULL,
    proposed_method_json TEXT NOT NULL CHECK (json_valid(proposed_method_json)),
    source_evidence_json TEXT NOT NULL CHECK (json_valid(source_evidence_json)),
    exposure_summary TEXT NOT NULL DEFAULT '',
    conflict_summary TEXT NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL,
    reviewed_at_ms INTEGER,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE,
    UNIQUE (task_run_id, kind)
);

CREATE TABLE IF NOT EXISTS saved_methods (
    method_id TEXT PRIMARY KEY CHECK (method_id GLOB 'method_*'),
    source_offer_id TEXT NOT NULL,
    project_id TEXT,
    name TEXT NOT NULL,
    summary TEXT NOT NULL,
    current_version INTEGER NOT NULL CHECK (current_version > 0),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    use_count INTEGER NOT NULL DEFAULT 0 CHECK (use_count >= 0),
    successful_use_count INTEGER NOT NULL DEFAULT 0 CHECK (successful_use_count >= 0),
    intervention_count INTEGER NOT NULL DEFAULT 0 CHECK (intervention_count >= 0),
    deleted_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (source_offer_id) REFERENCES learning_offers(offer_id),
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS saved_method_versions (
    method_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    method_json TEXT NOT NULL CHECK (json_valid(method_json)),
    source_task_run_id TEXT NOT NULL,
    change_summary TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (method_id, version),
    FOREIGN KEY (method_id) REFERENCES saved_methods(method_id) ON DELETE CASCADE,
    FOREIGN KEY (source_task_run_id) REFERENCES task_runs(task_run_id)
);

CREATE INDEX IF NOT EXISTS idx_learning_offers_task ON learning_offers(task_run_id, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_learning_offers_project ON learning_offers(project_id, status, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_saved_methods_project ON saved_methods(project_id, enabled, updated_at_ms DESC);
