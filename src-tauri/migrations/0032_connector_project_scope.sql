PRAGMA foreign_keys = ON;

ALTER TABLE connector_accounts
ADD COLUMN all_projects_enabled INTEGER NOT NULL DEFAULT 0
CHECK (all_projects_enabled IN (0,1));

ALTER TABLE connector_accounts
ADD COLUMN project_scope_reviewed_at_ms INTEGER;

UPDATE connector_accounts
SET project_scope_reviewed_at_ms = updated_at_ms
WHERE project_scope_reviewed_at_ms IS NULL;
