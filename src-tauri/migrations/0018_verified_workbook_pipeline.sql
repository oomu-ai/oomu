PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS workbook_records (
    artifact_id TEXT PRIMARY KEY CHECK (artifact_id GLOB 'artifact_*'),
    project_id TEXT NOT NULL CHECK (project_id GLOB 'project_*'),
    task_id TEXT NOT NULL CHECK (task_id GLOB 'task_*'),
    task_run_id TEXT NOT NULL CHECK (task_run_id GLOB 'taskrun_*'),
    title TEXT NOT NULL,
    current_revision INTEGER NOT NULL CHECK (current_revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS workbook_revisions (
    artifact_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    workbook_ir_json TEXT NOT NULL CHECK (json_valid(workbook_ir_json)),
    revision_instruction TEXT,
    status_code TEXT NOT NULL CHECK (status_code IN ('building','ready','needs_recalculation','check_required','failed')),
    xlsx_private_path TEXT,
    preview_manifest_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(preview_manifest_json)),
    verification_json TEXT CHECK (verification_json IS NULL OR json_valid(verification_json)),
    manifest_json TEXT CHECK (manifest_json IS NULL OR json_valid(manifest_json)),
    manifest_signature_json TEXT CHECK (manifest_signature_json IS NULL OR json_valid(manifest_signature_json)),
    xlsx_sha256 TEXT CHECK (xlsx_sha256 IS NULL OR length(xlsx_sha256) = 64),
    xlsx_bytes INTEGER CHECK (xlsx_bytes IS NULL OR xlsx_bytes > 0),
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    last_error TEXT,
    review_event_status_code TEXT NOT NULL DEFAULT 'pending' CHECK (review_event_status_code IN ('pending','recorded')),
    review_event_last_error TEXT,
    PRIMARY KEY (artifact_id, revision),
    FOREIGN KEY (artifact_id) REFERENCES workbook_records(artifact_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS workbook_source_links (
    artifact_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    sheet_id TEXT NOT NULL,
    cell_address TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    evidence_ref TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (artifact_id, revision, sheet_id, cell_address, source_ref, evidence_ref),
    FOREIGN KEY (artifact_id, revision) REFERENCES workbook_revisions(artifact_id, revision) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS workbook_exports (
    export_id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    destination_hash TEXT NOT NULL CHECK (length(destination_hash) = 64),
    xlsx_sha256 TEXT NOT NULL CHECK (length(xlsx_sha256) = 64),
    created_at_ms INTEGER NOT NULL,
    status_code TEXT NOT NULL DEFAULT 'pending' CHECK (status_code IN ('pending','committed','failed')),
    completed_at_ms INTEGER,
    last_error TEXT,
    FOREIGN KEY (artifact_id, revision) REFERENCES workbook_revisions(artifact_id, revision) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workbook_records_project ON workbook_records(project_id, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_workbook_records_task ON workbook_records(task_run_id, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_workbook_revisions_status ON workbook_revisions(status_code, created_at_ms);

CREATE TABLE IF NOT EXISTS workbook_template_imports (
    template_token TEXT PRIMARY KEY CHECK (template_token GLOB 'workbook_template_*'),
    project_id TEXT NOT NULL CHECK (project_id GLOB 'project_*'),
    task_id TEXT NOT NULL CHECK (task_id GLOB 'task_*'),
    task_run_id TEXT NOT NULL CHECK (task_run_id GLOB 'taskrun_*'),
    source_private_path TEXT NOT NULL,
    source_sha256 TEXT NOT NULL CHECK (length(source_sha256) = 64),
    source_bytes INTEGER NOT NULL CHECK (source_bytes > 0),
    sheet_manifest_json TEXT NOT NULL CHECK (json_valid(sheet_manifest_json)),
    status_code TEXT NOT NULL CHECK (status_code IN ('inspected','building','consumed','failed')),
    artifact_id TEXT,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE,
    FOREIGN KEY (artifact_id) REFERENCES workbook_records(artifact_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_workbook_template_imports_context ON workbook_template_imports(task_run_id, status_code, expires_at_ms);
