use super::*;

struct CancelTarget {
    status: String,
    claimed_at: Option<i64>,
    agent_id: String,
    provider_id: String,
    model_id: String,
    root_turn_id: String,
    parent_turn_id: Option<String>,
    turn_kind: String,
    terminal_message_id: Option<i64>,
}

impl PersistenceEngine {
    pub fn cancel_saved_chat_turn(
        &self,
        request: CancelSavedChatTurnRequest,
    ) -> rusqlite::Result<i64> {
        validate_cancel_request(&request)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let target = load_cancel_target(&transaction, &self.workspace_id, &request)?;
        if let Some(message_id) = existing_cancelled_message(&target)? {
            transaction.commit()?;
            return Ok(message_id);
        }
        ensure_target_is_cancellable(&target)?;
        let completed_at_ms = unix_time_ms();
        mark_turn_cancelled(
            &transaction,
            &self.workspace_id,
            &request,
            &target.status,
            completed_at_ms,
        )?;
        mark_turn_messages_cancelled(&transaction, &self.workspace_id, &request)?;
        let message_id = insert_cancelled_message(
            &transaction,
            &self.workspace_id,
            &request,
            &target,
            completed_at_ms,
        )?;
        transaction.commit()?;
        Ok(message_id)
    }
}

fn validate_cancel_request(request: &CancelSavedChatTurnRequest) -> rusqlite::Result<()> {
    if [
        request.session_id.as_str(),
        request.turn_id.as_str(),
        request.generation_token.as_str(),
        request.content.as_str(),
    ]
    .iter()
    .all(|value| !value.trim().is_empty())
    {
        return Ok(());
    }
    Err(cancel_error(
        "Cancelling saved work requires its exact turn identity.",
    ))
}

fn load_cancel_target(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    request: &CancelSavedChatTurnRequest,
) -> rusqlite::Result<CancelTarget> {
    transaction
        .query_row(
            "SELECT turns.status, turns.response_claimed_at_ms, turns.agent_id,
                    turns.provider_id, turns.model_id, turns.root_turn_id,
                    turns.parent_turn_id, turns.turn_kind,
                    (SELECT messages.id FROM chat_messages messages
                     WHERE messages.workspace_id = turns.workspace_id
                       AND messages.session_id = turns.session_id
                       AND json_extract(messages.metadata_json, '$.terminalResultForTurnId') = turns.turn_id
                     ORDER BY messages.id ASC LIMIT 1)
             FROM chat_turns turns
             JOIN chat_sessions sessions ON sessions.id = turns.session_id
               AND sessions.workspace_id = turns.workspace_id
               AND sessions.agent_id = turns.agent_id
             WHERE turns.workspace_id = ?1 AND turns.session_id = ?2
               AND turns.turn_id = ?3 AND turns.generation_token = ?4
               AND EXISTS (
                   SELECT 1 FROM chat_messages accepted
                   WHERE accepted.workspace_id = turns.workspace_id
                     AND accepted.session_id = turns.session_id
                     AND accepted.agent_id = turns.agent_id
                     AND accepted.role = 'user'
                     AND json_extract(accepted.metadata_json, '$.turnId') = turns.turn_id
                     AND json_extract(accepted.metadata_json, '$.generationToken') = turns.generation_token
               )",
            params![
                workspace_id,
                request.session_id.trim(),
                request.turn_id.trim(),
                request.generation_token.trim(),
            ],
            |row| {
                Ok(CancelTarget {
                    status: row.get(0)?,
                    claimed_at: row.get(1)?,
                    agent_id: row.get(2)?,
                    provider_id: row.get(3)?,
                    model_id: row.get(4)?,
                    root_turn_id: row.get(5)?,
                    parent_turn_id: row.get(6)?,
                    turn_kind: row.get(7)?,
                    terminal_message_id: row.get(8)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| cancel_error("The saved request no longer matches this recovery card."))
}

fn existing_cancelled_message(target: &CancelTarget) -> rusqlite::Result<Option<i64>> {
    match (target.status.as_str(), target.terminal_message_id) {
        ("cancelled", Some(message_id)) => Ok(Some(message_id)),
        (_, Some(_)) => Err(cancel_error(
            "The saved request already finished with a different result.",
        )),
        _ => Ok(None),
    }
}

fn ensure_target_is_cancellable(target: &CancelTarget) -> rusqlite::Result<()> {
    let safe = matches!(target.status.as_str(), "running" | "failed")
        && !(target.status == "running" && target.claimed_at.is_some());
    if safe {
        return Ok(());
    }
    Err(cancel_error(
        "The saved request is still changing and could not be cancelled safely.",
    ))
}

fn mark_turn_cancelled(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    request: &CancelSavedChatTurnRequest,
    previous_status: &str,
    completed_at_ms: i64,
) -> rusqlite::Result<()> {
    let changed = transaction.execute(
        "UPDATE chat_turns SET status = 'cancelled', completed_at_ms = ?5
         WHERE workspace_id = ?1 AND session_id = ?2 AND turn_id = ?3
           AND generation_token = ?4 AND status = ?6",
        params![
            workspace_id,
            request.session_id.trim(),
            request.turn_id.trim(),
            request.generation_token.trim(),
            completed_at_ms,
            previous_status,
        ],
    )?;
    if changed == 1 {
        return Ok(());
    }
    Err(cancel_error(
        "The saved request changed before cancellation could finish.",
    ))
}

fn mark_turn_messages_cancelled(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    request: &CancelSavedChatTurnRequest,
) -> rusqlite::Result<()> {
    transaction.execute(
        "UPDATE chat_messages
         SET metadata_json = json_set(COALESCE(metadata_json, '{}'), '$.turnState', 'cancelled')
         WHERE workspace_id = ?1 AND session_id = ?2
           AND json_extract(metadata_json, '$.turnId') = ?3",
        params![
            workspace_id,
            request.session_id.trim(),
            request.turn_id.trim()
        ],
    )?;
    Ok(())
}

fn insert_cancelled_message(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    request: &CancelSavedChatTurnRequest,
    target: &CancelTarget,
    completed_at_ms: i64,
) -> rusqlite::Result<i64> {
    let metadata = json!({
        "turnId": request.turn_id.trim(),
        "generationToken": request.generation_token.trim(),
        "sessionId": request.session_id.trim(),
        "agentId": target.agent_id,
        "rootTurnId": target.root_turn_id,
        "parentTurnId": target.parent_turn_id,
        "turnKind": target.turn_kind,
        "turnState": "cancelled",
        "terminalResultForTurnId": request.turn_id.trim(),
    });
    transaction.execute(
        "INSERT INTO chat_messages (
            workspace_id, session_id, agent_id, role, content, provider_id, model_id,
            metadata_json, is_compacted, compaction_type, timestamp_ms
         ) VALUES (?1, ?2, ?3, 'system', ?4, ?5, ?6, ?7, 0, 'raw', ?8)",
        params![
            workspace_id,
            request.session_id.trim(),
            target.agent_id,
            request.content.trim(),
            target.provider_id,
            target.model_id,
            metadata.to_string(),
            completed_at_ms,
        ],
    )?;
    Ok(transaction.last_insert_rowid())
}

fn cancel_error(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted_turn(name: &str) -> (std::path::PathBuf, PersistenceEngine, ChatSessionRecord) {
        let root = std::env::temp_dir().join(format!("oomu-chat-cancel-{name}-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: format!("agent-{name}"),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: None,
                dynamic_routing_override: Some(true),
                workspace_id: None,
            })
            .unwrap();
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: format!("turn-{name}"),
                generation_token: format!("generation-{name}"),
                parent_turn_id: None,
                root_turn_id: format!("turn-{name}"),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                message: "Keep the messages before this request.".to_string(),
            })
            .unwrap();
        (root, engine, session)
    }

    #[test]
    fn saved_auto_route_cancel_is_exact_durable_and_idempotent() {
        let (root, engine, session) = accepted_turn("cancel");
        let request = CancelSavedChatTurnRequest {
            session_id: session.id.clone(),
            turn_id: "turn-cancel".to_string(),
            generation_token: "generation-cancel".to_string(),
            content: "This request was cancelled before it finished.".to_string(),
        };

        let first = engine.cancel_saved_chat_turn(request.clone()).unwrap();
        assert_eq!(engine.cancel_saved_chat_turn(request).unwrap(), first);
        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].content,
            "Keep the messages before this request."
        );
        let metadata: Value = serde_json::from_str(
            messages[1]
                .metadata_json
                .as_deref()
                .expect("terminal metadata"),
        )
        .unwrap();
        assert_eq!(metadata["terminalResultForTurnId"], "turn-cancel");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_auto_route_cancel_rejects_a_different_generation_without_mutation() {
        let (root, engine, session) = accepted_turn("mismatch");
        let result = engine.cancel_saved_chat_turn(CancelSavedChatTurnRequest {
            session_id: session.id.clone(),
            turn_id: "turn-mismatch".to_string(),
            generation_token: "generation-wrong".to_string(),
            content: "This request was cancelled before it finished.".to_string(),
        });

        assert!(result.is_err());
        assert_eq!(engine.select_chat_messages(&session.id).unwrap().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
