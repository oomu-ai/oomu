CREATE TABLE remote_pairing_challenges (
    challenge_id TEXT PRIMARY KEY NOT NULL,
    secret_hash TEXT NOT NULL,
    qr_payload TEXT NOT NULL,
    requested_scopes_json TEXT NOT NULL CHECK(json_valid(requested_scopes_json)),
    allowed_project_ids_json TEXT NOT NULL CHECK(json_valid(allowed_project_ids_json)),
    pending_device_label TEXT,
    pending_public_key TEXT,
    response_received_at_ms INTEGER,
    expires_at_ms INTEGER NOT NULL,
    consumed_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE remote_devices (
    remote_device_id TEXT PRIMARY KEY NOT NULL,
    label TEXT NOT NULL,
    public_key TEXT NOT NULL UNIQUE,
    allowed_project_ids_json TEXT NOT NULL CHECK(json_valid(allowed_project_ids_json)),
    scopes_json TEXT NOT NULL CHECK(json_valid(scopes_json)),
    paired_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    last_used_at_ms INTEGER,
    revoked_at_ms INTEGER,
    key_generation INTEGER NOT NULL DEFAULT 1 CHECK(key_generation > 0)
);

CREATE TABLE remote_commands (
    command_id TEXT PRIMARY KEY NOT NULL,
    remote_device_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    task_run_id TEXT,
    command_kind TEXT NOT NULL,
    nonce TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    expected_task_sequence INTEGER,
    payload_sha256 TEXT NOT NULL,
    signer_public_key TEXT NOT NULL,
    signature TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued','accepted','rejected','expired','completed')),
    outcome_code TEXT,
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
    received_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE(remote_device_id, nonce),
    FOREIGN KEY(remote_device_id) REFERENCES remote_devices(remote_device_id)
);
CREATE INDEX idx_remote_commands_device_received ON remote_commands(remote_device_id, received_at_ms DESC);

CREATE TABLE remote_artifact_grants (
    grant_id TEXT PRIMARY KEY NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    remote_device_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    artifact_format TEXT NOT NULL CHECK(artifact_format IN ('docx','pdf')),
    private_path TEXT NOT NULL,
    artifact_sha256 TEXT NOT NULL,
    redaction_state TEXT NOT NULL,
    protected INTEGER NOT NULL DEFAULT 0,
    expires_at_ms INTEGER NOT NULL,
    opened_at_ms INTEGER,
    revoked_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY(remote_device_id) REFERENCES remote_devices(remote_device_id)
);

CREATE TABLE remote_audit_receipts (
    receipt_id TEXT PRIMARY KEY NOT NULL,
    remote_device_id TEXT NOT NULL,
    command_id TEXT,
    receipt_kind TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
    signature TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
