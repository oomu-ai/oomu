use rusqlite::Connection;

const SQL: &str = include_str!("../../migrations/0034_chat_session_attention.sql");
pub(super) const DESCRIPTOR: super::MigrationDescriptor =
    super::sql(34, "0034_chat_session_attention", SQL);

pub(super) fn apply(connection: &Connection) -> rusqlite::Result<()> {
    super::add_column_if_missing(
        connection,
        "chat_sessions",
        "unread_completion",
        "ALTER TABLE chat_sessions ADD COLUMN unread_completion INTEGER NOT NULL DEFAULT 0 CHECK(unread_completion IN (0, 1))",
    )?;
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_chat_sessions_unread_completion
         ON chat_sessions(workspace_id, unread_completion, updated_at_ms DESC);",
    )
}
