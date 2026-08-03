ALTER TABLE execution_instances RENAME TO execution_instances_before_approval_gateway;

CREATE TABLE execution_instances (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    workflow_version INTEGER NOT NULL CHECK (workflow_version > 0),
    status TEXT NOT NULL CHECK (
        status IN ('Pending', 'Running', 'AwaitingApproval', 'Completed', 'Failed')
    ),
    active_node_id TEXT,
    input_payload_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(input_payload_json)),
    output_payload_json TEXT CHECK (
        output_payload_json IS NULL OR json_valid(output_payload_json)
    ),
    node_payloads_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(node_payloads_json)),
    memory_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(memory_json)),
    selected_edges_json TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(selected_edges_json)),
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

INSERT INTO execution_instances (
    id, workflow_id, workflow_version, status, active_node_id, input_payload_json,
    output_payload_json, node_payloads_json, memory_json, selected_edges_json,
    pause_context_json, error_json, execution_latency_ms, prompt_tokens,
    completion_tokens, total_tokens, created_at_ms, started_at_ms, updated_at_ms,
    completed_at_ms, encryption_state
)
SELECT
    id, workflow_id, workflow_version,
    CASE status WHEN 'Paused' THEN 'AwaitingApproval' ELSE status END,
    active_node_id, input_payload_json, output_payload_json, node_payloads_json,
    '{}', '[]', pause_context_json, error_json, execution_latency_ms,
    prompt_tokens, completion_tokens, total_tokens, created_at_ms, started_at_ms,
    updated_at_ms, completed_at_ms, encryption_state
FROM execution_instances_before_approval_gateway;

DROP TABLE execution_instances_before_approval_gateway;

CREATE INDEX idx_execution_instances_status_updated
    ON execution_instances(status, updated_at_ms DESC);
CREATE INDEX idx_execution_instances_workflow
    ON execution_instances(workflow_id, workflow_version, created_at_ms DESC);
