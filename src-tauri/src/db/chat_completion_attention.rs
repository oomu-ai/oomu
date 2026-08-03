use super::*;

impl PersistenceEngine {
    pub fn record_chat_completion_attention(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> rusqlite::Result<(i64, bool)> {
        let session_id = session_id.trim();
        let turn_id = turn_id.trim();
        if session_id.is_empty() || turn_id.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat completion attention requires session and turn ids".to_string(),
            ));
        }
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let terminal_turns: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM chat_turns
             WHERE turn_id = ?1 AND session_id = ?2 AND workspace_id = ?3
               AND status IN ('completed', 'failed', 'escalated')",
            params![turn_id, session_id, &self.workspace_id],
            |row| row.get(0),
        )?;
        if terminal_turns != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat completion attention requires one matching terminal turn".to_string(),
            ));
        }
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO chat_completion_attention_receipts
             (workspace_id, session_id, turn_id, created_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![&self.workspace_id, session_id, turn_id, unix_time_ms()],
        )? == 1;
        if inserted {
            transaction.execute(
                "UPDATE chat_sessions SET unread_completion = 1
                 WHERE id = ?1 AND workspace_id = ?2",
                params![session_id, &self.workspace_id],
            )?;
        }
        let unread_count = transaction.query_row(
            "SELECT COUNT(*) FROM chat_sessions
             WHERE workspace_id = ?1 AND unread_completion = 1",
            params![&self.workspace_id],
            |row| row.get(0),
        )?;
        transaction.commit()?;
        Ok((unread_count, inserted))
    }
}
