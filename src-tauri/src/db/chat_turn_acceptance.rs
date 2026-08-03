use super::*;

#[path = "chat_turn_cancel.rs"]
mod cancel;
#[path = "chat_turn_acceptance_contract.rs"]
mod contract;
#[path = "chat_turn_resume.rs"]
mod resume;
pub(crate) use contract::CompleteClaimedChatTurnRequest;
pub use contract::{
    AbandonAcceptedChatTurnRequest, AcceptChatTurnRequest, AcceptedChatTurn,
    CancelSavedChatTurnRequest, FinalizeAcceptedChatTurnRequest,
};

struct AcceptedUserMessage {
    message_id: i64,
    session_was_empty_before_acceptance: bool,
}

fn validate_claimed_completion(
    request: &mut CompleteClaimedChatTurnRequest,
) -> rusqlite::Result<()> {
    canonicalize_terminal_content(&request.role, &mut request.content);
    if matches!(request.role.as_str(), "assistant" | "system")
        && !request.content.trim().is_empty()
        && matches!(
            request.status.as_str(),
            "completed" | "failed" | "cancelled" | "escalated"
        )
        && !request.message_provider_id.trim().is_empty()
        && !request.message_model_id.trim().is_empty()
        && !request.session_provider_id.trim().is_empty()
        && !request.session_model_id.trim().is_empty()
    {
        return Ok(());
    }
    Err(rusqlite::Error::InvalidParameterName(
        "invalid claimed chat turn completion".to_string(),
    ))
}

fn validate_accepted_finalization(
    request: &mut FinalizeAcceptedChatTurnRequest,
) -> rusqlite::Result<()> {
    canonicalize_terminal_content(&request.role, &mut request.content);
    if matches!(request.role.as_str(), "assistant" | "system")
        && !request.content.trim().is_empty()
        && matches!(
            request.status.as_str(),
            "completed" | "failed" | "cancelled" | "escalated"
        )
    {
        return Ok(());
    }
    Err(rusqlite::Error::InvalidParameterName(
        "invalid accepted chat turn finalization".to_string(),
    ))
}

fn canonicalize_terminal_content(role: &str, content: &mut String) {
    if role == "assistant" {
        *content = assistant_content::canonicalize_assistant_content(content);
    }
}

type AcceptedTurnAbandonState = (
    String,
    Option<i64>,
    String,
    String,
    Option<i64>,
    Option<String>,
);

fn accepted_turn_abandon_state(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    context: &ChatTurnPersistenceContext,
) -> rusqlite::Result<Option<AcceptedTurnAbandonState>> {
    transaction
        .query_row(
            "SELECT turns.status, turns.response_claimed_at_ms, turns.provider_id, turns.model_id,
                    (SELECT messages.id FROM chat_messages messages
                     WHERE messages.workspace_id = turns.workspace_id
                       AND messages.session_id = turns.session_id
                       AND json_extract(messages.metadata_json, '$.terminalResultForTurnId') = turns.turn_id
                     ORDER BY messages.id ASC LIMIT 1),
                    (SELECT json_extract(messages.metadata_json, '$.turnState')
                     FROM chat_messages messages
                     WHERE messages.workspace_id = turns.workspace_id
                       AND messages.session_id = turns.session_id
                       AND messages.role = 'user'
                       AND json_extract(messages.metadata_json, '$.turnId') = turns.turn_id
                       AND json_extract(messages.metadata_json, '$.generationToken') = turns.generation_token
                     ORDER BY messages.id ASC LIMIT 1)
             FROM chat_turns turns
             JOIN chat_sessions sessions
               ON sessions.id = turns.session_id
              AND sessions.workspace_id = turns.workspace_id
              AND sessions.agent_id = turns.agent_id
             WHERE turns.turn_id = ?1 AND turns.generation_token = ?2
               AND turns.workspace_id = ?3 AND turns.session_id = ?4
               AND turns.agent_id = ?5
               AND (
                    (turns.provider_id = ?6 AND turns.model_id = ?7)
                    OR (
                        lower(?6) = 'dynamic' AND lower(?7) = 'dynamic'
                        AND turns.response_claimed_at_ms IS NOT NULL
                        AND (
                            (lower(sessions.provider_id) = 'dynamic' AND lower(sessions.model_id) = 'dynamic')
                            OR COALESCE(sessions.dynamic_routing_override, 0) = 1
                        )
                    )
               )
               AND turns.root_turn_id = ?8 AND turns.turn_kind = ?9
               AND COALESCE(turns.parent_turn_id, '') = COALESCE(?10, '')",
            params![
                context.turn_id,
                context.generation_token,
                workspace_id,
                context.session_id,
                context.agent_id,
                context.provider_id,
                context.model_id,
                context.root_turn_id,
                context.turn_kind,
                context.parent_turn_id,
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
}

impl PersistenceEngine {
    pub fn accept_chat_turn(
        &self,
        request: AcceptChatTurnRequest,
    ) -> rusqlite::Result<AcceptedChatTurn> {
        if request.message.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "accepted chat turns require a user message".to_string(),
            ));
        }
        let context = request.persistence_context();
        self.begin_or_validate_running_chat_turn(&context)?;
        match self.ensure_chat_turn_user_message_acceptance(&context, &request.message, "accepted")
        {
            Ok(accepted_message) => Ok(AcceptedChatTurn {
                turn_id: context.turn_id,
                message_id: accepted_message.message_id,
                accepted: true,
                session_was_empty_before_acceptance: accepted_message
                    .session_was_empty_before_acceptance,
            }),
            Err(error) => {
                let _ = self.finish_chat_turn(&context, "failed");
                Err(error)
            }
        }
    }

    pub fn abandon_accepted_chat_turn(
        &self,
        request: AbandonAcceptedChatTurnRequest,
    ) -> rusqlite::Result<Option<i64>> {
        if request.content.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "abandoned chat turns require a terminal explanation".to_string(),
            ));
        }
        let context = request.persistence_context();
        validate_chat_turn_context_fields(&context)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &context.session_id, &self.workspace_id)?;
        let completed_at_ms = unix_time_ms();
        let state = accepted_turn_abandon_state(&transaction, &workspace_id, &context)?;
        let Some((
            status,
            response_claimed_at_ms,
            stored_provider_id,
            stored_model_id,
            existing_terminal,
            user_turn_state,
        )) = state
        else {
            transaction.commit()?;
            return Ok(None);
        };
        if status == "running" && response_claimed_at_ms.is_some() {
            transaction.commit()?;
            return Ok(None);
        }
        if status == "failed"
            && response_claimed_at_ms.is_none()
            && existing_terminal.is_none()
            && user_turn_state.as_deref() == Some("interrupted")
        {
            transaction.commit()?;
            return Ok(None);
        }
        if !matches!(status.as_str(), "running" | "failed") {
            transaction.commit()?;
            return Ok(None);
        }
        if status == "running" {
            let changed = transaction.execute(
                "UPDATE chat_turns
                 SET status = 'failed', completed_at_ms = ?10
                 WHERE turn_id = ?1 AND generation_token = ?2
                   AND workspace_id = ?3 AND session_id = ?4 AND agent_id = ?5
                   AND provider_id = ?6 AND model_id = ?7
                   AND root_turn_id = ?8 AND turn_kind = ?9
                   AND COALESCE(parent_turn_id, '') = COALESCE(?11, '')
                   AND status = 'running' AND response_claimed_at_ms IS NULL",
                params![
                    context.turn_id,
                    context.generation_token,
                    workspace_id,
                    context.session_id,
                    context.agent_id,
                    context.provider_id,
                    context.model_id,
                    context.root_turn_id,
                    context.turn_kind,
                    completed_at_ms,
                    context.parent_turn_id,
                ],
            )?;
            if changed != 1 {
                return Err(rusqlite::Error::InvalidParameterName(
                    "accepted chat turn abandonment lost its atomic claim".to_string(),
                ));
            }
        }
        transaction.execute(
            "UPDATE chat_messages
             SET metadata_json = json_set(
                 COALESCE(metadata_json, '{}'), '$.turnState', 'failed'
             )
             WHERE workspace_id = ?1 AND session_id = ?2
               AND json_extract(metadata_json, '$.turnId') = ?3",
            params![workspace_id, context.session_id, context.turn_id,],
        )?;
        if let Some(message_id) = existing_terminal {
            transaction.commit()?;
            return Ok(Some(message_id));
        }
        let metadata = json!({
            "turnId": context.turn_id,
            "generationToken": context.generation_token,
            "sessionId": context.session_id,
            "agentId": context.agent_id,
            "rootTurnId": context.root_turn_id,
            "parentTurnId": context.parent_turn_id,
            "turnKind": context.turn_kind,
            "turnState": "failed",
            "terminalResultForTurnId": context.turn_id,
        });
        transaction.execute(
            "INSERT INTO chat_messages (
                workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms
             ) VALUES (?1, ?2, ?3, 'system', ?4, ?5, ?6, ?7, 0, 'raw', ?8)",
            params![
                workspace_id,
                context.session_id,
                context.agent_id,
                request.content.trim(),
                stored_provider_id,
                stored_model_id,
                metadata.to_string(),
                completed_at_ms,
            ],
        )?;
        let message_id = transaction.last_insert_rowid();
        transaction.commit()?;
        Ok(Some(message_id))
    }

    pub(crate) fn complete_claimed_chat_turn(
        &self,
        mut request: CompleteClaimedChatTurnRequest,
    ) -> rusqlite::Result<i64> {
        validate_claimed_completion(&mut request)?;
        validate_chat_turn_context_fields(&request.context)?;
        let metadata = request.metadata.as_object_mut().ok_or_else(|| {
            rusqlite::Error::InvalidParameterName(
                "claimed chat turn metadata must be an object".to_string(),
            )
        })?;
        metadata.insert(
            "turnState".to_string(),
            Value::String(request.status.clone()),
        );
        metadata.insert(
            "terminalResultForTurnId".to_string(),
            Value::String(request.context.turn_id.clone()),
        );

        let context = &request.context;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &context.session_id, &self.workspace_id)?;
        let state = transaction.query_row(
            "SELECT turns.status, turns.response_claimed_at_ms,
                    (SELECT messages.id FROM chat_messages messages
                     WHERE messages.workspace_id = turns.workspace_id
                       AND messages.session_id = turns.session_id
                       AND json_extract(messages.metadata_json, '$.terminalResultForTurnId') = turns.turn_id
                     ORDER BY messages.id ASC LIMIT 1)
             FROM chat_turns turns
             JOIN chat_sessions sessions
               ON sessions.id = turns.session_id
              AND sessions.workspace_id = turns.workspace_id
              AND sessions.agent_id = turns.agent_id
             WHERE turns.turn_id = ?1 AND turns.generation_token = ?2
               AND turns.workspace_id = ?3 AND turns.session_id = ?4
               AND turns.agent_id = ?5 AND turns.provider_id = ?6 AND turns.model_id = ?7
               AND turns.root_turn_id = ?8 AND turns.turn_kind = ?9
               AND COALESCE(turns.parent_turn_id, '') = COALESCE(?10, '')",
            params![
                context.turn_id,
                context.generation_token,
                workspace_id,
                context.session_id,
                context.agent_id,
                context.provider_id,
                context.model_id,
                context.root_turn_id,
                context.turn_kind,
                context.parent_turn_id,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )?;
        if state.0 == request.status {
            return state.2.ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(
                    "completed chat turn is missing its terminal result".to_string(),
                )
            });
        }
        if state.0 != "running" || state.1.is_none() || state.2.is_some() {
            return Err(rusqlite::Error::InvalidParameterName(
                "claimed chat turn is not eligible for atomic completion".to_string(),
            ));
        }
        let completed_at_ms = unix_time_ms();
        let changed = transaction.execute(
            "UPDATE chat_turns
             SET status = ?1, completed_at_ms = ?2
             WHERE turn_id = ?3 AND generation_token = ?4
               AND workspace_id = ?5 AND session_id = ?6 AND agent_id = ?7
               AND provider_id = ?8 AND model_id = ?9
               AND root_turn_id = ?10 AND turn_kind = ?11
               AND COALESCE(parent_turn_id, '') = COALESCE(?12, '')
               AND status = 'running' AND response_claimed_at_ms IS NOT NULL",
            params![
                request.status,
                completed_at_ms,
                context.turn_id,
                context.generation_token,
                workspace_id,
                context.session_id,
                context.agent_id,
                context.provider_id,
                context.model_id,
                context.root_turn_id,
                context.turn_kind,
                context.parent_turn_id,
            ],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "claimed chat turn completion lost its atomic claim".to_string(),
            ));
        }
        transaction.execute(
            "UPDATE chat_messages
             SET metadata_json = json_set(COALESCE(metadata_json, '{}'), '$.turnState', ?3)
             WHERE workspace_id = ?1 AND session_id = ?2
               AND json_extract(metadata_json, '$.turnId') = ?4",
            params![
                workspace_id,
                context.session_id,
                request.status,
                context.turn_id,
            ],
        )?;
        transaction.execute(
            "INSERT INTO chat_messages (
                workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 'raw', ?9)",
            params![
                workspace_id,
                context.session_id,
                context.agent_id,
                request.role,
                request.content,
                request.message_provider_id,
                request.message_model_id,
                request.metadata.to_string(),
                completed_at_ms,
            ],
        )?;
        let message_id = transaction.last_insert_rowid();
        let title = request
            .session_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let session_changed = if let Some(title) = title {
            transaction.execute(
                "UPDATE chat_sessions
                 SET title = CASE WHEN title_source = 'user' THEN title ELSE ?1 END,
                     title_source = CASE WHEN title_source = 'user' THEN title_source ELSE 'auto' END,
                     provider_id = ?2, model_id = ?3, updated_at_ms = ?4
                 WHERE id = ?5 AND workspace_id = ?6 AND agent_id = ?7",
                params![
                    title,
                    request.session_provider_id,
                    request.session_model_id,
                    completed_at_ms,
                    context.session_id,
                    workspace_id,
                    context.agent_id,
                ],
            )?
        } else {
            transaction.execute(
                "UPDATE chat_sessions
                 SET provider_id = ?1, model_id = ?2, updated_at_ms = ?3
                 WHERE id = ?4 AND workspace_id = ?5 AND agent_id = ?6",
                params![
                    request.session_provider_id,
                    request.session_model_id,
                    completed_at_ms,
                    context.session_id,
                    workspace_id,
                    context.agent_id,
                ],
            )?
        };
        if session_changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "claimed chat turn session completion did not match".to_string(),
            ));
        }
        transaction.commit()?;
        Ok(message_id)
    }

    #[cfg(test)]
    pub(crate) fn ensure_chat_turn_user_message(
        &self,
        context: &ChatTurnPersistenceContext,
        content: &str,
        turn_state: &str,
    ) -> rusqlite::Result<i64> {
        self.ensure_chat_turn_user_message_acceptance(context, content, turn_state)
            .map(|accepted| accepted.message_id)
    }

    fn ensure_chat_turn_user_message_acceptance(
        &self,
        context: &ChatTurnPersistenceContext,
        content: &str,
        turn_state: &str,
    ) -> rusqlite::Result<AcceptedUserMessage> {
        validate_chat_turn_context_fields(context)?;
        let metadata = json!({
            "turnId": context.turn_id,
            "generationToken": context.generation_token,
            "sessionId": context.session_id,
            "agentId": context.agent_id,
            "rootTurnId": context.root_turn_id,
            "parentTurnId": context.parent_turn_id,
            "turnKind": context.turn_kind,
            "turnState": turn_state,
        });
        self.ensure_chat_turn_user_message_acceptance_with_metadata(context, content, &metadata)
    }

    pub(crate) fn ensure_chat_turn_user_message_with_metadata(
        &self,
        context: &ChatTurnPersistenceContext,
        content: &str,
        metadata: &Value,
    ) -> rusqlite::Result<i64> {
        self.ensure_chat_turn_user_message_acceptance_with_metadata(context, content, metadata)
            .map(|accepted| accepted.message_id)
    }

    fn ensure_chat_turn_user_message_acceptance_with_metadata(
        &self,
        context: &ChatTurnPersistenceContext,
        content: &str,
        metadata: &Value,
    ) -> rusqlite::Result<AcceptedUserMessage> {
        validate_chat_turn_context_fields(context)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &context.session_id, &self.workspace_id)?;
        let existing = transaction
            .query_row(
                "SELECT id, COALESCE(
                    json_extract(metadata_json, '$.sessionWasEmptyBeforeAcceptance'), 0
                 ) FROM chat_messages
                 WHERE workspace_id = ?1 AND session_id = ?2 AND role = 'user'
                   AND json_extract(metadata_json, '$.turnId') = ?3
                 ORDER BY id ASC LIMIT 1",
                params![workspace_id, context.session_id, context.turn_id],
                |row| {
                    Ok(AcceptedUserMessage {
                        message_id: row.get(0)?,
                        session_was_empty_before_acceptance: row.get(1)?,
                    })
                },
            )
            .optional()?;
        if let Some(accepted_message) = existing {
            transaction.execute(
                "UPDATE chat_messages
                 SET metadata_json = json_patch(COALESCE(metadata_json, '{}'), ?2)
                 WHERE id = ?1",
                params![accepted_message.message_id, metadata.to_string()],
            )?;
            transaction.commit()?;
            return Ok(accepted_message);
        }
        let session_was_empty_before_acceptance = transaction.query_row(
            "SELECT NOT EXISTS(
                SELECT 1 FROM chat_messages WHERE workspace_id = ?1 AND session_id = ?2
             )",
            params![workspace_id, context.session_id],
            |row| row.get::<_, bool>(0),
        )?;
        let mut metadata = metadata.clone();
        metadata["sessionWasEmptyBeforeAcceptance"] =
            Value::Bool(session_was_empty_before_acceptance);
        transaction.execute(
            "INSERT INTO chat_messages (
                workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms
             ) VALUES (?1, ?2, ?3, 'user', ?4, ?5, ?6, ?7, 0, 'raw', ?8)",
            params![
                workspace_id,
                context.session_id,
                context.agent_id,
                content,
                context.provider_id,
                context.model_id,
                metadata.to_string(),
                unix_time_ms(),
            ],
        )?;
        let message_id = transaction.last_insert_rowid();
        transaction.commit()?;
        Ok(AcceptedUserMessage {
            message_id,
            session_was_empty_before_acceptance,
        })
    }

    pub fn finalize_accepted_chat_turn(
        &self,
        mut request: FinalizeAcceptedChatTurnRequest,
    ) -> rusqlite::Result<i64> {
        validate_accepted_finalization(&mut request)?;
        let context = request.persistence_context();
        validate_chat_turn_context_fields(&context)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &context.session_id, &self.workspace_id)?;
        let state = transaction
            .query_row(
                "SELECT turns.status, turns.provider_id, turns.model_id, turns.response_claimed_at_ms,
                        (SELECT messages.id FROM chat_messages messages
                         WHERE messages.workspace_id = turns.workspace_id
                           AND messages.session_id = turns.session_id
                           AND json_extract(messages.metadata_json, '$.terminalResultForTurnId') = turns.turn_id
                         ORDER BY messages.id ASC LIMIT 1)
                 FROM chat_turns turns
                 JOIN chat_sessions sessions
                   ON sessions.id = turns.session_id
                  AND sessions.workspace_id = turns.workspace_id
                  AND sessions.agent_id = turns.agent_id
                 WHERE turns.turn_id = ?1 AND turns.generation_token = ?2
                   AND turns.workspace_id = ?3 AND turns.session_id = ?4
                   AND turns.agent_id = ?5
                   AND (
                        (turns.provider_id = ?6 AND turns.model_id = ?7)
                        OR (
                            lower(?6) = 'dynamic' AND lower(?7) = 'dynamic'
                            AND (
                                (lower(sessions.provider_id) = 'dynamic' AND lower(sessions.model_id) = 'dynamic')
                                OR COALESCE(sessions.dynamic_routing_override, 0) = 1
                            )
                        )
                   )
                   AND turns.root_turn_id = ?8 AND turns.turn_kind = ?9
                   AND COALESCE(turns.parent_turn_id, '') = COALESCE(?10, '')",
                params![
                    context.turn_id,
                    context.generation_token,
                    workspace_id,
                    context.session_id,
                    context.agent_id,
                    context.provider_id,
                    context.model_id,
                    context.root_turn_id,
                    context.turn_kind,
                    context.parent_turn_id,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            status,
            stored_provider_id,
            stored_model_id,
            response_claimed_at_ms,
            existing_terminal,
        )) = state
        else {
            return Err(rusqlite::Error::InvalidParameterName(
                "accepted chat turn does not match its immutable context".to_string(),
            ));
        };
        if status == "running" && response_claimed_at_ms.is_some() {
            return Err(rusqlite::Error::InvalidParameterName(
                "claimed chat response is still running".to_string(),
            ));
        }
        if status != "running" && status != request.status {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat turn already has a different terminal status".to_string(),
            ));
        }
        let completed_at_ms = unix_time_ms();
        if status == "running" {
            let changed = transaction.execute(
                "UPDATE chat_turns
                 SET status = ?1, completed_at_ms = ?2
                 WHERE turn_id = ?3 AND generation_token = ?4
                   AND workspace_id = ?5 AND session_id = ?6 AND agent_id = ?7
                   AND provider_id = ?8 AND model_id = ?9
                   AND root_turn_id = ?10 AND turn_kind = ?11
                   AND COALESCE(parent_turn_id, '') = COALESCE(?12, '')
                   AND status = 'running' AND response_claimed_at_ms IS NULL",
                params![
                    request.status,
                    completed_at_ms,
                    context.turn_id,
                    context.generation_token,
                    workspace_id,
                    context.session_id,
                    context.agent_id,
                    stored_provider_id,
                    stored_model_id,
                    context.root_turn_id,
                    context.turn_kind,
                    context.parent_turn_id,
                ],
            )?;
            if changed != 1 {
                return Err(rusqlite::Error::InvalidParameterName(
                    "accepted chat turn finalization lost its atomic claim".to_string(),
                ));
            }
        }
        transaction.execute(
            "UPDATE chat_messages
             SET metadata_json = json_set(COALESCE(metadata_json, '{}'), '$.turnState', ?3)
             WHERE workspace_id = ?1 AND session_id = ?2
               AND json_extract(metadata_json, '$.turnId') = ?4",
            params![
                workspace_id,
                context.session_id,
                request.status,
                context.turn_id,
            ],
        )?;
        if let Some(message_id) = existing_terminal {
            transaction.commit()?;
            return Ok(message_id);
        }
        let metadata = json!({
            "turnId": context.turn_id,
            "generationToken": context.generation_token,
            "sessionId": context.session_id,
            "agentId": context.agent_id,
            "rootTurnId": context.root_turn_id,
            "parentTurnId": context.parent_turn_id,
            "turnKind": context.turn_kind,
            "turnState": request.status,
            "terminalResultForTurnId": context.turn_id,
        });
        transaction.execute(
            "INSERT INTO chat_messages (
                workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 'raw', ?9)",
            params![
                workspace_id,
                context.session_id,
                context.agent_id,
                request.role,
                request.content,
                stored_provider_id,
                stored_model_id,
                metadata.to_string(),
                completed_at_ms,
            ],
        )?;
        let message_id = transaction.last_insert_rowid();
        transaction.commit()?;
        Ok(message_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptance_persists_exactly_one_user_message_before_response_claim() {
        let root = std::env::temp_dir().join(format!("oomu-chat-acceptance-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-a".to_string(),
                provider_id: "provider-a".to_string(),
                model_id: "model-a".to_string(),
                title: Some("Acceptance".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let request = AcceptChatTurnRequest {
            turn_id: "turn-a".to_string(),
            generation_token: "generation-a".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-a".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
            message: "  exact user prompt  ".to_string(),
        };

        let first = engine.accept_chat_turn(request.clone()).unwrap();
        let retry = engine.accept_chat_turn(request).unwrap();
        assert_eq!(first.message_id, retry.message_id);
        assert!(first.session_was_empty_before_acceptance);
        assert!(retry.session_was_empty_before_acceptance);
        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "  exact user prompt  ");
        assert!(messages[0]
            .metadata_json
            .as_deref()
            .unwrap()
            .contains("\"turnState\":\"accepted\""));
        assert!(messages[0]
            .metadata_json
            .as_deref()
            .unwrap()
            .contains("\"sessionWasEmptyBeforeAcceptance\":true"));
        let second = engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: "turn-b".to_string(),
                generation_token: "generation-b".to_string(),
                parent_turn_id: None,
                root_turn_id: "turn-b".to_string(),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: session.provider_id.clone(),
                model_id: session.model_id.clone(),
                message: "second user prompt".to_string(),
            })
            .unwrap();
        assert!(!second.session_was_empty_before_acceptance);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_keeps_an_accepted_turn_visible_and_marks_it_interrupted() {
        let root = std::env::temp_dir().join(format!("oomu-chat-recovery-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-recovery".to_string(),
                provider_id: "provider-recovery".to_string(),
                model_id: "model-recovery".to_string(),
                title: Some("Recovery".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: "turn-recovery".to_string(),
                generation_token: "generation-recovery".to_string(),
                parent_turn_id: None,
                root_turn_id: "turn-recovery".to_string(),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: session.provider_id.clone(),
                model_id: session.model_id.clone(),
                message: "Do not lose this turn.".to_string(),
            })
            .unwrap();

        engine.mark_interrupted_actions().unwrap();

        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Do not lose this turn.");
        assert!(messages[0]
            .metadata_json
            .as_deref()
            .unwrap()
            .contains("\"turnState\":\"interrupted\""));
        let status: String = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT status FROM chat_turns WHERE turn_id = 'turn-recovery'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "failed");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_clarification_is_persisted_once_on_the_accepted_turn() {
        let root = std::env::temp_dir().join(format!("oomu-chat-finalize-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-finalize".to_string(),
                provider_id: "provider-finalize".to_string(),
                model_id: "model-finalize".to_string(),
                title: Some("Finalize".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let accepted = AcceptChatTurnRequest {
            turn_id: "turn-finalize".to_string(),
            generation_token: "generation-finalize".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-finalize".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
            message: "Write it into that folder as Markdown.".to_string(),
        };
        engine.accept_chat_turn(accepted).unwrap();
        let result = FinalizeAcceptedChatTurnRequest {
            turn_id: "turn-finalize".to_string(),
            generation_token: "generation-finalize".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-finalize".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
            role: "assistant".to_string(),
            content: "What should I name the Markdown file?".to_string(),
            status: "escalated".to_string(),
        };
        let first = engine.finalize_accepted_chat_turn(result.clone()).unwrap();
        let retry = engine.finalize_accepted_chat_turn(result).unwrap();
        assert_eq!(first, retry);
        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "What should I name the Markdown file?");
        assert!(messages[0]
            .metadata_json
            .as_deref()
            .unwrap()
            .contains("\"turnState\":\"escalated\""));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn abandoned_unclaimed_turn_is_failed_atomically_and_idempotently() {
        let root = std::env::temp_dir().join(format!("oomu-chat-abandon-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-abandon".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Abandon".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            })
            .unwrap();
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: "turn-abandon".to_string(),
                generation_token: "generation-abandon".to_string(),
                parent_turn_id: None,
                root_turn_id: "turn-abandon".to_string(),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                message: "Do the complete task.".to_string(),
            })
            .unwrap();
        let request = AbandonAcceptedChatTurnRequest {
            turn_id: "turn-abandon".to_string(),
            generation_token: "generation-abandon".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-abandon".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            content: "OOMU couldn't start this reply safely. Try again.".to_string(),
        };

        let first = engine
            .abandon_accepted_chat_turn(request.clone())
            .unwrap()
            .expect("unclaimed turn is abandoned");
        let retry = engine
            .abandon_accepted_chat_turn(request)
            .unwrap()
            .expect("idempotent retry returns the terminal message");
        assert_eq!(first, retry);

        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[0]
            .metadata_json
            .as_deref()
            .unwrap()
            .contains("\"turnState\":\"failed\""));
        assert_eq!(
            messages[1].content,
            "OOMU couldn't start this reply safely. Try again."
        );
        let status: String = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT status FROM chat_turns WHERE turn_id = 'turn-abandon'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "failed");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn abandonment_never_steals_a_claimed_response() {
        let root = std::env::temp_dir().join(format!("oomu-chat-claimed-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-claimed".to_string(),
                provider_id: "provider-claimed".to_string(),
                model_id: "model-claimed".to_string(),
                title: Some("Claimed".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let accepted = AcceptChatTurnRequest {
            turn_id: "turn-claimed".to_string(),
            generation_token: "generation-claimed".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-claimed".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
            message: "Keep answering.".to_string(),
        };
        engine.accept_chat_turn(accepted).unwrap();
        let context = ChatTurnPersistenceContext {
            turn_id: "turn-claimed".to_string(),
            generation_token: "generation-claimed".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-claimed".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
        };
        engine.begin_or_claim_chat_turn_response(&context).unwrap();

        let abandoned = engine
            .abandon_accepted_chat_turn(AbandonAcceptedChatTurnRequest {
                turn_id: context.turn_id.clone(),
                generation_token: context.generation_token.clone(),
                parent_turn_id: None,
                root_turn_id: context.root_turn_id.clone(),
                turn_kind: context.turn_kind.clone(),
                session_id: context.session_id.clone(),
                agent_id: context.agent_id.clone(),
                provider_id: context.provider_id.clone(),
                model_id: context.model_id.clone(),
                content: "Should not be written.".to_string(),
            })
            .unwrap();
        assert_eq!(abandoned, None);
        assert_eq!(engine.select_chat_messages(&session.id).unwrap().len(), 1);
        let (status, claimed): (String, Option<i64>) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT status, response_claimed_at_ms FROM chat_turns WHERE turn_id = 'turn-claimed'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "running");
        assert!(claimed.is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn frontend_finalization_never_steals_a_claimed_static_response() {
        let root =
            std::env::temp_dir().join(format!("oomu-chat-claimed-finalize-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-claimed-finalize".to_string(),
                provider_id: "provider-claimed-finalize".to_string(),
                model_id: "model-claimed-finalize".to_string(),
                title: Some("Claimed finalization".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let context = ChatTurnPersistenceContext {
            turn_id: "turn-claimed-finalize".to_string(),
            generation_token: "generation-claimed-finalize".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-claimed-finalize".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
        };
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: context.turn_id.clone(),
                generation_token: context.generation_token.clone(),
                parent_turn_id: None,
                root_turn_id: context.root_turn_id.clone(),
                turn_kind: context.turn_kind.clone(),
                session_id: context.session_id.clone(),
                agent_id: context.agent_id.clone(),
                provider_id: context.provider_id.clone(),
                model_id: context.model_id.clone(),
                message: "Keep the claimed response alive.".to_string(),
            })
            .unwrap();
        engine.begin_or_claim_chat_turn_response(&context).unwrap();

        let error = engine
            .finalize_accepted_chat_turn(FinalizeAcceptedChatTurnRequest {
                turn_id: context.turn_id.clone(),
                generation_token: context.generation_token.clone(),
                parent_turn_id: None,
                root_turn_id: context.root_turn_id.clone(),
                turn_kind: context.turn_kind.clone(),
                session_id: context.session_id.clone(),
                agent_id: context.agent_id.clone(),
                provider_id: context.provider_id.clone(),
                model_id: context.model_id.clone(),
                role: "system".to_string(),
                content: "Should not replace the active response.".to_string(),
                status: "failed".to_string(),
            })
            .expect_err("frontend finalization must not steal a claimed response");
        assert!(error.to_string().contains("claimed chat response"));
        assert_eq!(engine.select_chat_messages(&session.id).unwrap().len(), 1);
        let status: String = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT status FROM chat_turns WHERE turn_id = 'turn-claimed-finalize'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "running");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn abandonment_repairs_a_failed_turn_missing_its_terminal_message() {
        let root = std::env::temp_dir().join(format!("oomu-chat-repair-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-repair".to_string(),
                provider_id: "provider-repair".to_string(),
                model_id: "model-repair".to_string(),
                title: Some("Repair".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: "turn-repair".to_string(),
                generation_token: "generation-repair".to_string(),
                parent_turn_id: None,
                root_turn_id: "turn-repair".to_string(),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: session.provider_id.clone(),
                model_id: session.model_id.clone(),
                message: "Preserve this request.".to_string(),
            })
            .unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE chat_turns SET status = 'failed', completed_at_ms = 1 WHERE turn_id = 'turn-repair'",
                [],
            )
            .unwrap();

        let terminal = engine
            .abandon_accepted_chat_turn(AbandonAcceptedChatTurnRequest {
                turn_id: "turn-repair".to_string(),
                generation_token: "generation-repair".to_string(),
                parent_turn_id: None,
                root_turn_id: "turn-repair".to_string(),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: session.provider_id.clone(),
                model_id: session.model_id.clone(),
                content: "OOMU could not safely continue this reply.".to_string(),
            })
            .unwrap()
            .expect("missing terminal is repaired");
        let retry = engine
            .abandon_accepted_chat_turn(AbandonAcceptedChatTurnRequest {
                turn_id: "turn-repair".to_string(),
                generation_token: "generation-repair".to_string(),
                parent_turn_id: None,
                root_turn_id: "turn-repair".to_string(),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: session.provider_id.clone(),
                model_id: session.model_id.clone(),
                content: "OOMU could not safely continue this reply.".to_string(),
            })
            .unwrap();
        assert_eq!(retry, Some(terminal));
        assert_eq!(engine.select_chat_messages(&session.id).unwrap().len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dynamic_frontend_context_repairs_a_failed_concrete_route() {
        let root = std::env::temp_dir().join(format!("oomu-chat-route-repair-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-route-repair".to_string(),
                provider_id: "configured-provider".to_string(),
                model_id: "configured-model".to_string(),
                title: Some("Route repair".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            })
            .unwrap();
        let dynamic = ChatTurnPersistenceContext {
            turn_id: "turn-route-repair".to_string(),
            generation_token: "generation-route-repair".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-route-repair".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
        };
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: dynamic.turn_id.clone(),
                generation_token: dynamic.generation_token.clone(),
                parent_turn_id: None,
                root_turn_id: dynamic.root_turn_id.clone(),
                turn_kind: dynamic.turn_kind.clone(),
                session_id: dynamic.session_id.clone(),
                agent_id: dynamic.agent_id.clone(),
                provider_id: dynamic.provider_id.clone(),
                model_id: dynamic.model_id.clone(),
                message: "Keep this Auto-route turn lossless.".to_string(),
            })
            .unwrap();
        let mut concrete = dynamic.clone();
        concrete.provider_id = "local_model".to_string();
        concrete.model_id = "gemma-route-repair".to_string();
        engine.begin_or_claim_chat_turn_response(&concrete).unwrap();
        engine.finish_chat_turn(&concrete, "failed").unwrap();

        let terminal = engine
            .abandon_accepted_chat_turn(AbandonAcceptedChatTurnRequest {
                turn_id: dynamic.turn_id.clone(),
                generation_token: dynamic.generation_token.clone(),
                parent_turn_id: None,
                root_turn_id: dynamic.root_turn_id.clone(),
                turn_kind: dynamic.turn_kind.clone(),
                session_id: dynamic.session_id.clone(),
                agent_id: dynamic.agent_id.clone(),
                provider_id: dynamic.provider_id.clone(),
                model_id: dynamic.model_id.clone(),
                content: "The selected model could not finish this reply.".to_string(),
            })
            .unwrap()
            .expect("dynamic alias repairs concrete failed route");
        let (provider_id, model_id): (String, String) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT provider_id, model_id FROM chat_messages WHERE id = ?1",
                params![terminal],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(provider_id, concrete.provider_id);
        assert_eq!(model_id, concrete.model_id);
        assert_eq!(engine.select_chat_messages(&session.id).unwrap().len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claimed_response_message_session_and_status_commit_atomically() {
        let root = std::env::temp_dir().join(format!("oomu-chat-complete-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-complete".to_string(),
                provider_id: "provider-complete".to_string(),
                model_id: "model-complete".to_string(),
                title: Some("Before".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let context = ChatTurnPersistenceContext {
            turn_id: "turn-complete".to_string(),
            generation_token: "generation-complete".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-complete".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
        };
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: context.turn_id.clone(),
                generation_token: context.generation_token.clone(),
                parent_turn_id: None,
                root_turn_id: context.root_turn_id.clone(),
                turn_kind: context.turn_kind.clone(),
                session_id: context.session_id.clone(),
                agent_id: context.agent_id.clone(),
                provider_id: context.provider_id.clone(),
                model_id: context.model_id.clone(),
                message: "Persist one response.".to_string(),
            })
            .unwrap();
        engine.begin_or_claim_chat_turn_response(&context).unwrap();
        let request = CompleteClaimedChatTurnRequest {
            context: context.clone(),
            role: "assistant".to_string(),
            content: "One durable response.".to_string(),
            message_provider_id: context.provider_id.clone(),
            message_model_id: context.model_id.clone(),
            metadata: json!({"turnId": context.turn_id}),
            session_title: Some("Persist one response.".to_string()),
            session_provider_id: context.provider_id.clone(),
            session_model_id: context.model_id.clone(),
            status: "completed".to_string(),
        };
        let first = engine.complete_claimed_chat_turn(request.clone()).unwrap();
        let retry = engine.complete_claimed_chat_turn(request).unwrap();
        assert_eq!(first, retry);
        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[1]
            .metadata_json
            .as_deref()
            .unwrap()
            .contains("terminalResultForTurnId"));
        let (status, title): (String, String) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT turns.status, sessions.title
                 FROM chat_turns turns JOIN chat_sessions sessions ON sessions.id = turns.session_id
                 WHERE turns.turn_id = 'turn-complete'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "completed");
        assert_eq!(title, "Persist one response.");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finalize_and_abandon_commit_exactly_one_terminal_result() {
        use std::sync::Barrier;

        let root = std::env::temp_dir().join(format!("oomu-chat-terminal-race-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-terminal-race".to_string(),
                provider_id: "provider-terminal-race".to_string(),
                model_id: "model-terminal-race".to_string(),
                title: Some("Terminal race".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: "turn-terminal-race".to_string(),
                generation_token: "generation-terminal-race".to_string(),
                parent_turn_id: None,
                root_turn_id: "turn-terminal-race".to_string(),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: session.provider_id.clone(),
                model_id: session.model_id.clone(),
                message: "Complete exactly once.".to_string(),
            })
            .unwrap();
        let finalize = FinalizeAcceptedChatTurnRequest {
            turn_id: "turn-terminal-race".to_string(),
            generation_token: "generation-terminal-race".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-terminal-race".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
            role: "assistant".to_string(),
            content: "Completed.".to_string(),
            status: "completed".to_string(),
        };
        let abandon = AbandonAcceptedChatTurnRequest {
            turn_id: finalize.turn_id.clone(),
            generation_token: finalize.generation_token.clone(),
            parent_turn_id: None,
            root_turn_id: finalize.root_turn_id.clone(),
            turn_kind: finalize.turn_kind.clone(),
            session_id: finalize.session_id.clone(),
            agent_id: finalize.agent_id.clone(),
            provider_id: finalize.provider_id.clone(),
            model_id: finalize.model_id.clone(),
            content: "Stopped safely.".to_string(),
        };
        let barrier = std::sync::Arc::new(Barrier::new(3));
        let finalize_worker = {
            let engine = engine.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                engine.finalize_accepted_chat_turn(finalize)
            })
        };
        let abandon_worker = {
            let engine = engine.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                engine.abandon_accepted_chat_turn(abandon)
            })
        };
        barrier.wait();
        let _ = finalize_worker.join().unwrap();
        let _ = abandon_worker.join().unwrap();

        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages
                .iter()
                .filter(|message| message
                    .metadata_json
                    .as_deref()
                    .is_some_and(|metadata| metadata.contains("terminalResultForTurnId")))
                .count(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
