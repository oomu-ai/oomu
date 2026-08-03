CREATE TABLE presentation_records (
    presentation_id TEXT PRIMARY KEY NOT NULL,
    artifact_id TEXT NOT NULL UNIQUE,
    project_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    title TEXT NOT NULL,
    current_revision INTEGER NOT NULL CHECK(current_revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_presentation_records_project_updated
    ON presentation_records(project_id, updated_at_ms DESC);

CREATE TABLE presentation_revisions (
    presentation_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    presentation_ir_json TEXT NOT NULL,
    scope_code TEXT NOT NULL,
    change_summary TEXT NOT NULL,
    status_code TEXT NOT NULL,
    pptx_private_path TEXT,
    preview_manifest_json TEXT,
    verification_json TEXT,
    pptx_sha256 TEXT,
    pptx_bytes INTEGER,
    manifest_json TEXT,
    manifest_signature_json TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    last_error TEXT,
    PRIMARY KEY(presentation_id, revision),
    FOREIGN KEY(presentation_id) REFERENCES presentation_records(presentation_id) ON DELETE CASCADE
);
CREATE INDEX idx_presentation_revisions_status
    ON presentation_revisions(status_code, completed_at_ms);

CREATE TABLE presentation_source_links (
    presentation_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    slide_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    evidence_ref TEXT NOT NULL,
    evidence_class TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(presentation_id, revision, slide_id, object_id, source_ref, evidence_ref),
    FOREIGN KEY(presentation_id, revision)
        REFERENCES presentation_revisions(presentation_id, revision) ON DELETE CASCADE
);

CREATE TABLE presentation_exports (
    export_id TEXT PRIMARY KEY NOT NULL,
    presentation_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    destination_name TEXT NOT NULL,
    pptx_sha256 TEXT NOT NULL,
    receipt_id TEXT NOT NULL,
    status_code TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    last_error TEXT,
    FOREIGN KEY(presentation_id, revision)
        REFERENCES presentation_revisions(presentation_id, revision)
);

CREATE TABLE presentation_template_imports (
    template_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    fingerprint_sha256 TEXT NOT NULL,
    private_path TEXT NOT NULL,
    inspection_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_presentation_template_fingerprint
    ON presentation_template_imports(project_id, fingerprint_sha256);
