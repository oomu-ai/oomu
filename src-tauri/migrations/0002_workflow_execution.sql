PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS workflow_blueprints (
    workflow_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    description TEXT NOT NULL DEFAULT '',
    visual_state_json TEXT NOT NULL CHECK (json_valid(visual_state_json)),
    workflow_ir_json TEXT CHECK (workflow_ir_json IS NULL OR json_valid(workflow_ir_json)),
    is_active INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms),
    compiled_at_ms INTEGER CHECK (compiled_at_ms IS NULL OR compiled_at_ms >= created_at_ms),
    encryption_state TEXT NOT NULL DEFAULT 'software_bound_aes256',
    PRIMARY KEY (workflow_id, version)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_blueprints_one_active
    ON workflow_blueprints(workflow_id)
    WHERE is_active = 1;
CREATE INDEX IF NOT EXISTS idx_workflow_blueprints_updated
    ON workflow_blueprints(updated_at_ms DESC);

CREATE TABLE IF NOT EXISTS compiled_instructions (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    workflow_version INTEGER NOT NULL CHECK (workflow_version > 0),
    node_id TEXT NOT NULL CHECK (length(trim(node_id)) > 0),
    node_kind TEXT NOT NULL CHECK (
        node_kind IN ('input', 'agent', 'router', 'permission', 'output')
    ),
    system_prompt TEXT NOT NULL DEFAULT '',
    input_variable_mappings_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(input_variable_mappings_json)),
    evaluation_protocol_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(evaluation_protocol_json)),
    compiler_model TEXT NOT NULL DEFAULT 'gemma-4-e2b-qat',
    compiler_version TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    encryption_state TEXT NOT NULL DEFAULT 'software_bound_aes256',
    FOREIGN KEY (workflow_id, workflow_version)
        REFERENCES workflow_blueprints(workflow_id, version)
        ON DELETE CASCADE,
    UNIQUE (workflow_id, workflow_version, node_id)
);

CREATE INDEX IF NOT EXISTS idx_compiled_instructions_workflow
    ON compiled_instructions(workflow_id, workflow_version);

CREATE TABLE IF NOT EXISTS execution_instances (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    workflow_version INTEGER NOT NULL CHECK (workflow_version > 0),
    status TEXT NOT NULL CHECK (
        status IN ('Pending', 'Running', 'Paused', 'Completed', 'Failed')
    ),
    active_node_id TEXT,
    input_payload_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(input_payload_json)),
    output_payload_json TEXT CHECK (
        output_payload_json IS NULL OR json_valid(output_payload_json)
    ),
    node_payloads_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(node_payloads_json)),
    pause_context_json TEXT CHECK (
        pause_context_json IS NULL OR json_valid(pause_context_json)
    ),
    error_json TEXT CHECK (error_json IS NULL OR json_valid(error_json)),
    execution_latency_ms INTEGER NOT NULL DEFAULT 0 CHECK (execution_latency_ms >= 0),
    prompt_tokens INTEGER NOT NULL DEFAULT 0 CHECK (prompt_tokens >= 0),
    completion_tokens INTEGER NOT NULL DEFAULT 0 CHECK (completion_tokens >= 0),
    total_tokens INTEGER NOT NULL DEFAULT 0 CHECK (
        total_tokens >= 0 AND total_tokens = prompt_tokens + completion_tokens
    ),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    started_at_ms INTEGER CHECK (started_at_ms IS NULL OR started_at_ms >= created_at_ms),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms),
    completed_at_ms INTEGER CHECK (
        completed_at_ms IS NULL OR completed_at_ms >= created_at_ms
    ),
    encryption_state TEXT NOT NULL DEFAULT 'software_bound_aes256',
    FOREIGN KEY (workflow_id, workflow_version)
        REFERENCES workflow_blueprints(workflow_id, version)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_execution_instances_status_updated
    ON execution_instances(status, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_execution_instances_workflow
    ON execution_instances(workflow_id, workflow_version, created_at_ms DESC);
