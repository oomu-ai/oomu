use super::{table_exists, unix_time_ms, workspace_id_for_chat_session, PersistenceEngine};
use rusqlite::params;

const RECOVERABLE_CHAT_DELETE_RETENTION_MS: i64 = 15_000;

impl PersistenceEngine {
    pub fn stage_chat_session_deletion_by_id(&self, session_id: &str) -> rusqlite::Result<bool> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?;
        let transaction = connection.transaction()?;
        let deleted_at_ms = unix_time_ms();
        let archived = transaction.execute(
            "
            INSERT INTO recoverable_chat_sessions (
                id, workspace_id, project_id, agent_id, title, title_source, provider_id,
                model_id, web_grounding_override, dynamic_routing_override, created_at_ms,
                updated_at_ms, encryption_state, deleted_at_ms, purge_after_ms
            )
            SELECT id, workspace_id, project_id, agent_id, title, title_source, provider_id,
                   model_id, web_grounding_override, dynamic_routing_override, created_at_ms,
                   updated_at_ms, encryption_state, ?3, ?4
            FROM chat_sessions
            WHERE id = ?1 AND workspace_id = ?2
            ",
            params![
                session_id,
                workspace_id,
                deleted_at_ms,
                deleted_at_ms + RECOVERABLE_CHAT_DELETE_RETENTION_MS
            ],
        )?;
        if archived == 0 {
            return Ok(false);
        }
        transaction.execute(
            "
            INSERT INTO recoverable_chat_messages (
                id, workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms, encryption_state
            )
            SELECT id, workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                   metadata_json, is_compacted, compaction_type, timestamp_ms, encryption_state
            FROM chat_messages
            WHERE session_id = ?1 AND workspace_id = ?2
            ",
            params![session_id, workspace_id],
        )?;
        transaction.execute(
            "
            UPDATE chat_turns
            SET status = 'cancelled', completed_at_ms = ?3
            WHERE session_id = ?1 AND workspace_id = ?2 AND status = 'running'
            ",
            params![session_id, workspace_id, unix_time_ms()],
        )?;
        if table_exists(&transaction, "taskflows")? {
            if table_exists(&transaction, "taskflow_steps")? {
                transaction.execute(
                    "
                    UPDATE taskflow_steps
                    SET status = 'cancelled'
                    WHERE flow_id IN (
                        SELECT flow_id FROM taskflows WHERE parent_session_id = ?1
                    )
                      AND status IN ('queued', 'active')
                    ",
                    params![session_id],
                )?;
            }
            transaction.execute(
                "
                UPDATE taskflows
                SET status = 'cancelled', updated_at_ms = ?2
                WHERE parent_session_id = ?1
                  AND status IN ('queued', 'active', 'failed', 'diagnostic', 'paused', 'secure_pause')
                ",
                params![session_id, unix_time_ms()],
            )?;
        }
        if table_exists(&transaction, "agent_executions")? {
            transaction.execute(
                "
                UPDATE agent_executions
                SET status = 'cancelled', updated_at_ms = ?2
                WHERE session_id = ?1 AND status = 'running'
                ",
                params![session_id, unix_time_ms()],
            )?;
        }
        transaction.execute(
            "DELETE FROM chat_messages WHERE session_id = ?1 AND workspace_id = ?2",
            params![session_id, workspace_id],
        )?;
        transaction.execute(
            "DELETE FROM message_queue WHERE session_id = ?1",
            params![session_id],
        )?;
        let removed = transaction.execute(
            "DELETE FROM chat_sessions WHERE id = ?1 AND workspace_id = ?2",
            params![session_id, workspace_id],
        )?;
        transaction.commit()?;
        Ok(removed > 0)
    }

    pub fn undo_chat_session_deletion_by_id(&self, session_id: &str) -> rusqlite::Result<bool> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let workspace_id = self.workspace_id.as_str();
        let transaction = connection.transaction()?;
        let restored = transaction.execute(
            "
            INSERT INTO chat_sessions (
                id, workspace_id, project_id, agent_id, title, title_source, provider_id,
                model_id, web_grounding_override, dynamic_routing_override, created_at_ms,
                updated_at_ms, encryption_state
            )
            SELECT id, workspace_id, project_id, agent_id, title, title_source, provider_id,
                   model_id, web_grounding_override, dynamic_routing_override, created_at_ms,
                   updated_at_ms, encryption_state
            FROM recoverable_chat_sessions
            WHERE id = ?1 AND workspace_id = ?2
            ",
            params![session_id, workspace_id],
        )?;
        if restored == 0 {
            return Ok(false);
        }
        transaction.execute(
            "
            INSERT INTO chat_messages (
                id, workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms, encryption_state
            )
            SELECT id, workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                   metadata_json, is_compacted, compaction_type, timestamp_ms, encryption_state
            FROM recoverable_chat_messages
            WHERE session_id = ?1 AND workspace_id = ?2
            ORDER BY id ASC
            ",
            params![session_id, workspace_id],
        )?;
        transaction.execute(
            "DELETE FROM recoverable_chat_sessions WHERE id = ?1 AND workspace_id = ?2",
            params![session_id, workspace_id],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn commit_chat_session_deletion_by_id(&self, session_id: &str) -> rusqlite::Result<bool> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let removed = connection.execute(
            "DELETE FROM recoverable_chat_sessions WHERE id = ?1 AND workspace_id = ?2",
            params![session_id, self.workspace_id.as_str()],
        )?;
        drop(connection);
        if removed > 0 {
            self.delete_auto_route_audit_for_session(session_id)?;
        }
        Ok(removed > 0)
    }

    pub fn delete_chat_session_by_id(&self, session_id: &str) -> rusqlite::Result<bool> {
        if !self.stage_chat_session_deletion_by_id(session_id)? {
            return Ok(false);
        }
        self.commit_chat_session_deletion_by_id(session_id)
    }

    pub(super) fn purge_expired_recoverable_chat_session_deletions(&self) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "SELECT id FROM recoverable_chat_sessions
             WHERE workspace_id = ?1 AND purge_after_ms <= ?2",
        )?;
        let expired = statement
            .query_map(params![self.workspace_id.as_str(), unix_time_ms()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        if expired.is_empty() {
            return Ok(());
        }
        for session_id in &expired {
            connection.execute(
                "DELETE FROM recoverable_chat_sessions WHERE id = ?1 AND workspace_id = ?2",
                params![session_id, self.workspace_id.as_str()],
            )?;
        }
        drop(connection);
        for session_id in expired {
            self.delete_auto_route_audit_for_session(&session_id)?;
        }
        Ok(())
    }
}
