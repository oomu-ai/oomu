ALTER TABLE chat_sessions
ADD COLUMN unread_completion INTEGER NOT NULL DEFAULT 0
CHECK(unread_completion IN (0, 1));

CREATE INDEX idx_chat_sessions_unread_completion
ON chat_sessions(workspace_id, unread_completion, updated_at_ms DESC);
