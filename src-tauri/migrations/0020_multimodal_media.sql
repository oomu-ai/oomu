CREATE TABLE media_assets (
    media_asset_id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    task_id TEXT,
    task_run_id TEXT,
    media_kind TEXT NOT NULL CHECK(media_kind IN ('audio','image','video')),
    mime_type TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    byte_length INTEGER NOT NULL CHECK(byte_length >= 0),
    source_kind TEXT NOT NULL,
    source_reference TEXT NOT NULL,
    width INTEGER,
    height INTEGER,
    duration_ms INTEGER,
    retention_mode TEXT NOT NULL CHECK(retention_mode IN ('task','project','until')),
    expires_at_ms INTEGER,
    redaction_state TEXT NOT NULL CHECK(redaction_state IN ('not_required','required','applied')),
    redaction_categories_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(redaction_categories_json)),
    routing_mode TEXT NOT NULL CHECK(routing_mode IN ('local_only','approved_providers')),
    provider_ids_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(provider_ids_json)),
    original_blob BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) ON DELETE CASCADE
);
CREATE INDEX idx_media_assets_project_created ON media_assets(project_id, created_at_ms DESC);
CREATE INDEX idx_media_assets_task ON media_assets(task_run_id, created_at_ms DESC);

CREATE TABLE media_asset_relationships (
    media_asset_id TEXT NOT NULL,
    related_media_asset_id TEXT NOT NULL,
    relationship TEXT NOT NULL CHECK(relationship IN ('source','derivative','transcript','thumbnail')),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(media_asset_id, related_media_asset_id, relationship),
    FOREIGN KEY(media_asset_id) REFERENCES media_assets(media_asset_id) ON DELETE CASCADE,
    FOREIGN KEY(related_media_asset_id) REFERENCES media_assets(media_asset_id) ON DELETE CASCADE
);

CREATE TABLE media_transcripts (
    media_asset_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    transcript TEXT NOT NULL,
    language TEXT NOT NULL,
    confidence REAL,
    timestamps_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(timestamps_json)),
    route_kind TEXT NOT NULL CHECK(route_kind IN ('local','provider','manual')),
    route_label TEXT NOT NULL,
    edited_by_user INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(media_asset_id, revision),
    FOREIGN KEY(media_asset_id) REFERENCES media_assets(media_asset_id) ON DELETE CASCADE
);

CREATE TABLE media_evidence (
    evidence_id TEXT PRIMARY KEY NOT NULL,
    media_asset_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    task_run_id TEXT,
    evidence_class TEXT NOT NULL,
    event_kind TEXT NOT NULL,
    detail_json TEXT NOT NULL CHECK(json_valid(detail_json)),
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY(media_asset_id) REFERENCES media_assets(media_asset_id) ON DELETE CASCADE
);

CREATE TABLE media_interpretations (
    media_asset_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    interpretation_kind TEXT NOT NULL CHECK(interpretation_kind IN ('local_vision','alt_text')),
    text TEXT NOT NULL,
    route_label TEXT NOT NULL,
    edited_by_user INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(media_asset_id, revision),
    FOREIGN KEY(media_asset_id) REFERENCES media_assets(media_asset_id) ON DELETE CASCADE
);
