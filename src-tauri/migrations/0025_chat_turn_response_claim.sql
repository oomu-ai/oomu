ALTER TABLE chat_turns ADD COLUMN response_claimed_at_ms INTEGER;

-- Every row created before this claim protocol represented a response path
-- that had already started. Mark it claimed so an upgrade can never replay
-- historical or interrupted turns.
UPDATE chat_turns
SET response_claimed_at_ms = created_at_ms
WHERE response_claimed_at_ms IS NULL;
