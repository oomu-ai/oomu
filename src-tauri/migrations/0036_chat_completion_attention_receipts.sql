CREATE TABLE chat_completion_attention_receipts (
    workspace_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(workspace_id, turn_id),
    FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(turn_id) REFERENCES chat_turns(turn_id) ON DELETE CASCADE
);

CREATE INDEX idx_chat_completion_attention_session
ON chat_completion_attention_receipts(workspace_id, session_id, created_at_ms DESC);
