PRAGMA foreign_keys = ON;

-- A recoverable deletion is deliberately outside every live chat and execution
-- table. The active session is still revoked immediately; only inert
-- conversation history remains available to the short Undo window.
CREATE TABLE recoverable_chat_sessions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    project_id TEXT,
    agent_id TEXT NOT NULL,
    title TEXT NOT NULL,
    title_source TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    web_grounding_override INTEGER,
    dynamic_routing_override INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    encryption_state TEXT NOT NULL DEFAULT '{}',
    deleted_at_ms INTEGER NOT NULL,
    purge_after_ms INTEGER NOT NULL
);

CREATE INDEX idx_recoverable_chat_sessions_expiry
ON recoverable_chat_sessions(workspace_id, purge_after_ms);

CREATE TABLE recoverable_chat_messages (
    id INTEGER PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    provider_id TEXT,
    model_id TEXT,
    metadata_json TEXT,
    is_compacted INTEGER NOT NULL DEFAULT 0,
    compaction_type TEXT,
    timestamp_ms INTEGER NOT NULL,
    encryption_state TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(session_id) REFERENCES recoverable_chat_sessions(id) ON DELETE CASCADE
);

CREATE INDEX idx_recoverable_chat_messages_session
ON recoverable_chat_messages(workspace_id, session_id, timestamp_ms, id);
