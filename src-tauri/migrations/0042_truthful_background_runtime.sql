ALTER TABLE background_service_state
ADD COLUMN requested_enabled INTEGER NOT NULL DEFAULT 0
    CHECK (requested_enabled IN (0, 1));

ALTER TABLE background_service_state
ADD COLUMN runtime_state TEXT NOT NULL DEFAULT 'off'
    CHECK (runtime_state IN ('off', 'turning_on', 'on_verified', 'needs_attention', 'turning_off'));

ALTER TABLE background_service_state
ADD COLUMN registration_state TEXT NOT NULL DEFAULT 'unregistered'
    CHECK (registration_state IN ('unregistered', 'registering', 'registered', 'requires_approval', 'unavailable', 'failed'));

ALTER TABLE background_service_state
ADD COLUMN registration_backend TEXT NOT NULL DEFAULT 'unknown'
    CHECK (registration_backend IN ('unknown', 'sm_app_service', 'supervised_process'));

ALTER TABLE background_service_state
ADD COLUMN registration_generation TEXT;

ALTER TABLE background_service_state
ADD COLUMN process_state TEXT NOT NULL DEFAULT 'absent'
    CHECK (process_state IN ('absent', 'starting', 'running', 'stopping'));

ALTER TABLE background_service_state
ADD COLUMN process_id INTEGER;

ALTER TABLE background_service_state
ADD COLUMN build_number INTEGER NOT NULL DEFAULT 0 CHECK (build_number >= 0);

ALTER TABLE background_service_state
ADD COLUMN build_identity TEXT NOT NULL DEFAULT 'unavailable';

ALTER TABLE background_service_state
ADD COLUMN profile_class TEXT NOT NULL DEFAULT 'unknown';

ALTER TABLE background_service_state
ADD COLUMN profile_generation TEXT NOT NULL DEFAULT '';

ALTER TABLE background_service_state
ADD COLUMN heartbeat_at_ms INTEGER;

ALTER TABLE background_service_state
ADD COLUMN heartbeat_expires_at_ms INTEGER;

ALTER TABLE background_service_state
ADD COLUMN menu_visible INTEGER NOT NULL DEFAULT 0 CHECK (menu_visible IN (0, 1));

UPDATE background_service_state
SET requested_enabled = user_enabled,
    runtime_state = CASE
        WHEN user_enabled = 1 THEN 'needs_attention'
        ELSE 'off'
    END,
    registration_state = CASE service_status
        WHEN 'active' THEN 'registered'
        WHEN 'requires_approval' THEN 'requires_approval'
        WHEN 'paused' THEN 'unregistered'
        WHEN 'unavailable' THEN 'unavailable'
        ELSE 'failed'
    END,
    process_state = 'absent',
    process_id = NULL,
    heartbeat_at_ms = NULL,
    heartbeat_expires_at_ms = NULL,
    menu_visible = 0;

CREATE TABLE background_runtime_receipts (
    receipt_id TEXT PRIMARY KEY,
    event_kind TEXT NOT NULL CHECK (event_kind IN (
        'requested_state_changed',
        'registration_started',
        'registration_verified',
        'registration_failed',
        'heartbeat_verified',
        'runtime_stopped',
        'attention_required',
        'shutdown_verified',
        'reconciliation_started',
        'reconciliation_verified',
        'reconciliation_failed',
        'menu_shown',
        'menu_hidden',
        'window_closed',
        'window_reopened',
        'quit_requested',
        'scheduled_postcondition_verified'
    )),
    outcome TEXT NOT NULL CHECK (outcome IN ('started', 'verified', 'attention', 'completed')),
    runtime_state TEXT NOT NULL CHECK (runtime_state IN ('off', 'turning_on', 'on_verified', 'needs_attention', 'turning_off')),
    requested_enabled INTEGER NOT NULL CHECK (requested_enabled IN (0, 1)),
    registration_generation TEXT,
    process_id INTEGER,
    build_number INTEGER NOT NULL CHECK (build_number >= 0),
    build_identity TEXT NOT NULL DEFAULT 'unavailable',
    profile_class TEXT NOT NULL,
    profile_generation TEXT NOT NULL,
    detail_code TEXT,
    subject_id_hash TEXT,
    result_digest TEXT,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_background_runtime_receipts_created
ON background_runtime_receipts(created_at_ms DESC);
