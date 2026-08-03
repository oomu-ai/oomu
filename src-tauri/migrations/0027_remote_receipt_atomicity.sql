PRAGMA foreign_keys = ON;

-- Existing receipts remain honest legacy rows. A NULL signer key means the
-- receipt predates the authoritative remote-command receipt contract.
ALTER TABLE remote_audit_receipts ADD COLUMN signer_public_key TEXT;

CREATE UNIQUE INDEX idx_remote_command_final_receipt
ON remote_audit_receipts(command_id)
WHERE command_id IS NOT NULL AND receipt_kind = 'remote_command';

CREATE TRIGGER validate_remote_command_receipt_insert
BEFORE INSERT ON remote_audit_receipts
WHEN NEW.receipt_kind = 'remote_command'
BEGIN
    SELECT CASE
        WHEN NEW.command_id IS NULL
          OR length(trim(COALESCE(NEW.signer_public_key, ''))) = 0
          OR length(trim(NEW.payload_sha256)) = 0
          OR length(trim(NEW.signature)) = 0
        THEN RAISE(ABORT, 'remote_command_receipt_incomplete')
    END;
END;

CREATE TRIGGER validate_remote_command_receipt_update
BEFORE UPDATE ON remote_audit_receipts
WHEN OLD.receipt_kind = 'remote_command' OR NEW.receipt_kind = 'remote_command'
BEGIN
    SELECT CASE
        WHEN OLD.receipt_kind <> 'remote_command'
        THEN RAISE(ABORT, 'remote_command_receipt_immutable')
        WHEN NEW.receipt_id IS NOT OLD.receipt_id
          OR NEW.receipt_kind IS NOT OLD.receipt_kind
          OR NEW.remote_device_id IS NOT OLD.remote_device_id
          OR NEW.payload_sha256 IS NOT OLD.payload_sha256
          OR NEW.signer_public_key IS NOT OLD.signer_public_key
          OR NEW.signature IS NOT OLD.signature
          OR NEW.created_at_ms IS NOT OLD.created_at_ms
          OR NOT (
              NEW.command_id IS OLD.command_id
              OR (OLD.command_id IS NOT NULL AND NEW.command_id IS NULL)
          )
        THEN RAISE(ABORT, 'remote_command_receipt_immutable')
    END;
END;

CREATE TABLE remote_effect_outbox (
    effect_id TEXT PRIMARY KEY NOT NULL,
    command_id TEXT NOT NULL UNIQUE,
    effect_kind TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    canonical_request_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','applied','failed')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
    last_error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    FOREIGN KEY(command_id) REFERENCES remote_commands(command_id) ON DELETE CASCADE
);

CREATE INDEX idx_remote_effect_outbox_pending
ON remote_effect_outbox(status, created_at_ms);
