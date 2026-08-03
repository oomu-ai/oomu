use rusqlite::Connection;

pub(super) const SQL: &str =
    include_str!("../../migrations/0036_chat_completion_attention_receipts.sql");
pub(super) const DESCRIPTOR: super::MigrationDescriptor =
    super::sql(36, "0036_chat_completion_attention_receipts", SQL);

pub(super) fn apply(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(SQL)
}
