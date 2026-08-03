PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS projects (
    project_id TEXT PRIMARY KEY CHECK (project_id GLOB 'project_*'),
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 120),
    description TEXT NOT NULL DEFAULT '',
    archived_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms)
);

CREATE TABLE IF NOT EXISTS project_sources (
    source_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('local_folder', 'knowledge_directory')),
    canonical_path TEXT NOT NULL,
    grant_reference TEXT NOT NULL,
    grant_state TEXT NOT NULL DEFAULT 'active'
        CHECK (grant_state IN ('active', 'revoked', 'unavailable')),
    indexing_state TEXT NOT NULL DEFAULT 'pending'
        CHECK (indexing_state IN ('pending', 'indexing', 'ready', 'failed', 'revoked')),
    file_count INTEGER NOT NULL DEFAULT 0 CHECK (file_count >= 0),
    last_indexed_at_ms INTEGER,
    failure_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(project_id, canonical_path),
    FOREIGN KEY(project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS project_instructions (
    project_id TEXT PRIMARY KEY,
    instructions TEXT NOT NULL DEFAULT '',
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS project_policy (
    project_id TEXT PRIMARY KEY,
    data_policy TEXT NOT NULL DEFAULT 'ask_before_cloud'
        CHECK (data_policy IN ('local_only', 'ask_before_cloud', 'allow_configured_cloud')),
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS project_policy_decisions (
    decision_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    task_id TEXT,
    destination_kind TEXT NOT NULL,
    destination_origin TEXT NOT NULL,
    data_classes_json TEXT NOT NULL CHECK (json_valid(data_classes_json)),
    decision TEXT NOT NULL CHECK (decision IN ('allowed', 'blocked', 'consent_required', 'consented')),
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

ALTER TABLE chat_sessions ADD COLUMN project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL;
ALTER TABLE workflows ADD COLUMN project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL;
ALTER TABLE workflow_blueprints ADD COLUMN project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL;
ALTER TABLE workflow_schedules ADD COLUMN project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL;
ALTER TABLE execution_instances ADD COLUMN project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL;
ALTER TABLE agent_executions ADD COLUMN project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL;
ALTER TABLE message_queue ADD COLUMN project_id TEXT REFERENCES projects(project_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_projects_updated ON projects(archived_at_ms, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_project_sources_project ON project_sources(project_id, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_chat_sessions_project ON chat_sessions(project_id, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_workflow_blueprints_project ON workflow_blueprints(project_id, updated_at_ms DESC);
