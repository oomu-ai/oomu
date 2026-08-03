PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS workflow_schedules (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL CHECK (length(trim(workflow_id)) > 0),
    workflow_version INTEGER CHECK (workflow_version IS NULL OR workflow_version > 0),
    label TEXT NOT NULL DEFAULT '',
    schedule_expression TEXT NOT NULL CHECK (length(trim(schedule_expression)) > 0),
    run_request_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(run_request_json)),
    is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    next_run_at_ms INTEGER CHECK (
        next_run_at_ms IS NULL OR next_run_at_ms >= 0
    ),
    claimed_at_ms INTEGER CHECK (
        claimed_at_ms IS NULL OR claimed_at_ms >= 0
    ),
    last_started_at_ms INTEGER CHECK (
        last_started_at_ms IS NULL OR last_started_at_ms >= 0
    ),
    last_completed_at_ms INTEGER CHECK (
        last_completed_at_ms IS NULL OR last_completed_at_ms >= 0
    ),
    last_status TEXT CHECK (
        last_status IS NULL OR last_status IN (
            'Pending',
            'Running',
            'AwaitingApproval',
            'Completed',
            'Failed'
        )
    ),
    last_error TEXT,
    last_instance_id TEXT,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms),
    encryption_state TEXT NOT NULL DEFAULT 'software_bound_aes256',
    FOREIGN KEY (workflow_id, workflow_version)
        REFERENCES workflow_blueprints(workflow_id, version)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workflow_schedules_due
    ON workflow_schedules(is_active, next_run_at_ms, claimed_at_ms);

CREATE INDEX IF NOT EXISTS idx_workflow_schedules_workflow
    ON workflow_schedules(workflow_id, workflow_version);
