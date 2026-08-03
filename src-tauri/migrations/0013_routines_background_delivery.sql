PRAGMA foreign_keys = ON;

ALTER TABLE workflow_schedules ADD COLUMN routine_timezone TEXT NOT NULL DEFAULT 'UTC';
ALTER TABLE workflow_schedules ADD COLUMN schedule_kind TEXT NOT NULL DEFAULT 'recurring' CHECK (schedule_kind IN ('one_shot','recurring'));
ALTER TABLE workflow_schedules ADD COLUMN active_window_start_minute INTEGER CHECK (active_window_start_minute BETWEEN 0 AND 1439);
ALTER TABLE workflow_schedules ADD COLUMN active_window_end_minute INTEGER CHECK (active_window_end_minute BETWEEN 0 AND 1439);
ALTER TABLE workflow_schedules ADD COLUMN missed_run_policy TEXT NOT NULL DEFAULT 'skip' CHECK (missed_run_policy IN ('skip','run_once','run_each'));
ALTER TABLE workflow_schedules ADD COLUMN missed_run_cap INTEGER NOT NULL DEFAULT 3 CHECK (missed_run_cap BETWEEN 1 AND 12);
ALTER TABLE workflow_schedules ADD COLUMN model_route_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(model_route_json));
ALTER TABLE workflow_schedules ADD COLUMN delivery_target_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(delivery_target_json));
ALTER TABLE workflow_schedules ADD COLUMN authority_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(authority_json));
ALTER TABLE workflow_schedules ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK (consecutive_failures >= 0);
ALTER TABLE workflow_schedules ADD COLUMN failure_threshold INTEGER NOT NULL DEFAULT 3 CHECK (failure_threshold BETWEEN 1 AND 10);
ALTER TABLE workflow_schedules ADD COLUMN paused_reason TEXT;

CREATE TABLE IF NOT EXISTS scheduler_owner_lease (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    owner_id TEXT NOT NULL,
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('foreground','background_service')),
    lease_epoch INTEGER NOT NULL,
    acquired_at_ms INTEGER NOT NULL,
    heartbeat_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS background_service_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    user_enabled INTEGER NOT NULL DEFAULT 0 CHECK (user_enabled IN (0,1)),
    service_status TEXT NOT NULL DEFAULT 'unavailable' CHECK (service_status IN ('active','paused','degraded','unavailable','requires_approval')),
    last_error_code TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS routine_authority_grants (
    grant_id TEXT PRIMARY KEY,
    schedule_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    action_name TEXT NOT NULL,
    arguments_hash TEXT NOT NULL,
    channel_platform TEXT,
    channel_owner_hash TEXT,
    expires_at_ms INTEGER NOT NULL,
    revoked_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY (schedule_id) REFERENCES workflow_schedules(id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS routine_delivery_receipts (
    receipt_id TEXT PRIMARY KEY,
    schedule_id TEXT NOT NULL,
    task_run_id TEXT,
    platform TEXT NOT NULL,
    destination_hash TEXT NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('started','blocked','failed','completed','approval')),
    state TEXT NOT NULL CHECK (state IN ('pending','delivered','failed')),
    provider_receipt_hash TEXT,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (schedule_id) REFERENCES workflow_schedules(id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS routine_runs (
    schedule_id TEXT NOT NULL,
    execution_instance_id TEXT NOT NULL UNIQUE,
    task_run_id TEXT,
    scheduled_for_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (schedule_id, execution_instance_id),
    FOREIGN KEY (schedule_id) REFERENCES workflow_schedules(id) ON DELETE CASCADE,
    FOREIGN KEY (execution_instance_id) REFERENCES execution_instances(id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS routine_remote_approvals (
    decision_code_hash TEXT PRIMARY KEY,
    schedule_id TEXT NOT NULL,
    execution_instance_id TEXT NOT NULL,
    task_run_id TEXT,
    node_id TEXT NOT NULL,
    action_name TEXT NOT NULL,
    arguments_hash TEXT NOT NULL,
    channel_platform TEXT NOT NULL,
    channel_owner_hash TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    decided_at_ms INTEGER,
    decision TEXT CHECK (decision IS NULL OR decision IN ('approve','reject')),
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY (schedule_id) REFERENCES workflow_schedules(id) ON DELETE CASCADE,
    FOREIGN KEY (execution_instance_id) REFERENCES execution_instances(id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_routines_project ON workflow_schedules(project_id, is_active, next_run_at_ms);
CREATE INDEX IF NOT EXISTS idx_routine_authority_scope ON routine_authority_grants(schedule_id, action_name, arguments_hash, expires_at_ms);
CREATE INDEX IF NOT EXISTS idx_routine_delivery_task ON routine_delivery_receipts(task_run_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_routine_runs_task ON routine_runs(task_run_id, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_routine_remote_approval_expiry ON routine_remote_approvals(expires_at_ms, decided_at_ms);
