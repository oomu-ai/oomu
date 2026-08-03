PRAGMA foreign_keys = ON;

ALTER TABLE remote_artifact_grants ADD COLUMN content_state TEXT
    CHECK(content_state IS NULL OR content_state IN (
        'full_content',
        'verified_redacted_derivative',
        'legacy_unverified'
    ));
ALTER TABLE remote_artifact_grants ADD COLUMN source_sha256 TEXT;
ALTER TABLE remote_artifact_grants ADD COLUMN transfer_sha256 TEXT;
ALTER TABLE remote_artifact_grants ADD COLUMN source_path TEXT;
ALTER TABLE remote_artifact_grants ADD COLUMN transfer_path TEXT;
ALTER TABLE remote_artifact_grants ADD COLUMN redaction_manifest_sha256 TEXT;
ALTER TABLE remote_artifact_grants ADD COLUMN approval_receipt_id TEXT;

-- Do not reinterpret an old claim as verified. These rows remain visible for
-- audit and revocation but the retrieval query deliberately excludes them.
UPDATE remote_artifact_grants
SET content_state = 'legacy_unverified',
    source_sha256 = artifact_sha256,
    transfer_sha256 = artifact_sha256,
    source_path = private_path,
    transfer_path = private_path
WHERE content_state IS NULL;

CREATE TRIGGER validate_remote_artifact_grant_insert
BEFORE INSERT ON remote_artifact_grants
BEGIN
    SELECT CASE
        WHEN NEW.content_state NOT IN ('full_content','verified_redacted_derivative')
          OR length(trim(COALESCE(NEW.source_sha256, ''))) = 0
          OR length(trim(COALESCE(NEW.transfer_sha256, ''))) = 0
          OR length(trim(COALESCE(NEW.source_path, ''))) = 0
          OR length(trim(COALESCE(NEW.transfer_path, ''))) = 0
          OR length(trim(COALESCE(NEW.approval_receipt_id, ''))) = 0
          OR NOT EXISTS (
              SELECT 1 FROM remote_audit_receipts
              WHERE receipt_id = NEW.approval_receipt_id
                AND receipt_kind = 'remote_artifact_approval'
          )
        THEN RAISE(ABORT, 'remote_artifact_grant_incomplete')
        WHEN NEW.content_state = 'full_content'
          AND (NEW.source_sha256 <> NEW.transfer_sha256 OR NEW.source_path <> NEW.transfer_path)
        THEN RAISE(ABORT, 'remote_artifact_full_content_mismatch')
        WHEN NEW.content_state = 'verified_redacted_derivative'
          AND length(trim(COALESCE(NEW.redaction_manifest_sha256, ''))) = 0
        THEN RAISE(ABORT, 'remote_artifact_derivative_manifest_required')
    END;
END;

CREATE TRIGGER validate_remote_artifact_grant_update
BEFORE UPDATE ON remote_artifact_grants
WHEN OLD.content_state IN ('full_content','verified_redacted_derivative','legacy_unverified')
  OR NEW.content_state IN ('full_content','verified_redacted_derivative')
BEGIN
    SELECT CASE
        WHEN OLD.content_state = 'legacy_unverified'
          AND NEW.content_state IS NOT OLD.content_state
        THEN RAISE(ABORT, 'remote_artifact_legacy_grant_immutable')
        WHEN OLD.content_state IN ('full_content','verified_redacted_derivative')
          AND (
              NEW.grant_id IS NOT OLD.grant_id
              OR NEW.token_hash IS NOT OLD.token_hash
              OR NEW.remote_device_id IS NOT OLD.remote_device_id
              OR NEW.project_id IS NOT OLD.project_id
              OR NEW.artifact_id IS NOT OLD.artifact_id
              OR NEW.artifact_format IS NOT OLD.artifact_format
              OR NEW.private_path IS NOT OLD.private_path
              OR NEW.artifact_sha256 IS NOT OLD.artifact_sha256
              OR NEW.redaction_state IS NOT OLD.redaction_state
              OR NEW.protected IS NOT OLD.protected
              OR NEW.expires_at_ms IS NOT OLD.expires_at_ms
              OR NEW.created_at_ms IS NOT OLD.created_at_ms
              OR NEW.content_state IS NOT OLD.content_state
              OR NEW.source_sha256 IS NOT OLD.source_sha256
              OR NEW.transfer_sha256 IS NOT OLD.transfer_sha256
              OR NEW.source_path IS NOT OLD.source_path
              OR NEW.transfer_path IS NOT OLD.transfer_path
              OR NEW.redaction_manifest_sha256 IS NOT OLD.redaction_manifest_sha256
              OR NEW.approval_receipt_id IS NOT OLD.approval_receipt_id
          )
        THEN RAISE(ABORT, 'remote_artifact_grant_immutable')
        WHEN NEW.content_state IN ('full_content','verified_redacted_derivative')
          AND (length(trim(COALESCE(NEW.source_sha256, ''))) = 0
          OR length(trim(COALESCE(NEW.transfer_sha256, ''))) = 0
          OR length(trim(COALESCE(NEW.source_path, ''))) = 0
          OR length(trim(COALESCE(NEW.transfer_path, ''))) = 0
          OR length(trim(COALESCE(NEW.approval_receipt_id, ''))) = 0
          OR NOT EXISTS (
              SELECT 1 FROM remote_audit_receipts
              WHERE receipt_id = NEW.approval_receipt_id
                AND receipt_kind = 'remote_artifact_approval'
          ))
        THEN RAISE(ABORT, 'remote_artifact_grant_incomplete')
        WHEN NEW.content_state = 'full_content'
          AND (NEW.source_sha256 <> NEW.transfer_sha256 OR NEW.source_path <> NEW.transfer_path)
        THEN RAISE(ABORT, 'remote_artifact_full_content_mismatch')
        WHEN NEW.content_state = 'verified_redacted_derivative'
          AND length(trim(COALESCE(NEW.redaction_manifest_sha256, ''))) = 0
        THEN RAISE(ABORT, 'remote_artifact_derivative_manifest_required')
    END;
END;

CREATE INDEX idx_remote_artifact_grants_retrievable
ON remote_artifact_grants(remote_device_id, token_hash, expires_at_ms)
WHERE content_state IN ('full_content','verified_redacted_derivative')
  AND opened_at_ms IS NULL
  AND revoked_at_ms IS NULL;
