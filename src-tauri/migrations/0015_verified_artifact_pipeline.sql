PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS artifact_records (
    artifact_id TEXT PRIMARY KEY CHECK (artifact_id GLOB 'artifact_*'),
    project_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    title TEXT NOT NULL,
    current_version INTEGER NOT NULL DEFAULT 0 CHECK (current_version >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(project_id) ON DELETE CASCADE,
    FOREIGN KEY (task_run_id) REFERENCES task_runs(task_run_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS artifact_versions (
    artifact_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    document_json TEXT NOT NULL CHECK (json_valid(document_json)),
    revision_instruction TEXT,
    status TEXT NOT NULL CHECK (status IN ('building','verifying','verified','failed')),
    docx_private_path TEXT,
    pdf_private_path TEXT,
    preview_manifest_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(preview_manifest_json)),
    verification_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(verification_json)),
    provenance_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(provenance_json)),
    manifest_json TEXT CHECK (manifest_json IS NULL OR json_valid(manifest_json)),
    manifest_signature_json TEXT CHECK (manifest_signature_json IS NULL OR json_valid(manifest_signature_json)),
    docx_sha256 TEXT,
    pdf_sha256 TEXT,
    docx_bytes INTEGER,
    pdf_bytes INTEGER,
    builder_identity TEXT NOT NULL,
    renderer_identity TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    last_error TEXT,
    PRIMARY KEY (artifact_id, version),
    FOREIGN KEY (artifact_id) REFERENCES artifact_records(artifact_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS artifact_source_links (
    artifact_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    source_ref TEXT NOT NULL,
    evidence_ref TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (artifact_id, version, source_ref, evidence_ref),
    FOREIGN KEY (artifact_id, version) REFERENCES artifact_versions(artifact_id, version) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS artifact_exports (
    export_id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('docx','pdf','both')),
    destination_hash TEXT NOT NULL,
    exported_hashes_json TEXT NOT NULL CHECK (json_valid(exported_hashes_json)),
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY (artifact_id, version) REFERENCES artifact_versions(artifact_id, version) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_artifacts_project ON artifact_records(project_id, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_artifacts_task ON artifact_records(task_run_id, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_artifact_versions_status ON artifact_versions(status, created_at_ms);
