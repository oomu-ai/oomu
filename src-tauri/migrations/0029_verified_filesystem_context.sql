PRAGMA foreign_keys = ON;

CREATE TABLE verified_filesystem_contexts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    project_id TEXT,
    source_turn_id TEXT NOT NULL,
    source_generation_token TEXT NOT NULL,
    operation TEXT NOT NULL CHECK(operation IN ('file_read','file_list','file_write')),
    canonical_path TEXT NOT NULL,
    target_kind TEXT NOT NULL CHECK(target_kind IN ('file','directory')),
    receipt_payload TEXT NOT NULL CHECK(json_valid(receipt_payload)),
    signature_json TEXT NOT NULL CHECK(json_valid(signature_json)),
    completed_at_ms INTEGER NOT NULL,
    result_status TEXT NOT NULL CHECK(result_status = 'completed'),
    encryption_state TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE,
    UNIQUE(session_id, source_turn_id, operation)
);

CREATE INDEX idx_verified_filesystem_context_session
ON verified_filesystem_contexts(workspace_id, session_id, completed_at_ms DESC, id DESC);

CREATE TABLE pending_contextual_file_actions (
    session_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    source_turn_id TEXT NOT NULL,
    directory_path TEXT NOT NULL,
    directory_receipt_digest TEXT NOT NULL,
    content_message_id INTEGER NOT NULL,
    content_digest TEXT NOT NULL,
    requested_format TEXT NOT NULL CHECK(requested_format IN ('md')),
    status TEXT NOT NULL CHECK(status = 'awaiting_filename'),
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(content_message_id) REFERENCES chat_messages(id) ON DELETE CASCADE
);
