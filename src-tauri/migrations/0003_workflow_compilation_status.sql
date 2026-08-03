ALTER TABLE workflow_blueprints
    ADD COLUMN compilation_status TEXT NOT NULL DEFAULT 'Draft'
    CHECK (compilation_status IN ('Draft', 'Compiling', 'Compiled', 'Failed'));

ALTER TABLE workflow_blueprints
    ADD COLUMN compilation_error TEXT;

CREATE INDEX IF NOT EXISTS idx_workflow_blueprints_compilation_status
    ON workflow_blueprints(compilation_status, updated_at_ms DESC);
