use super::*;

const AMBIENT_SEARCH_UNAVAILABLE_KEY: &str = "chat.search_errors.ambient_unavailable";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcceptedChatTurnCheckpointKind {
    WebGroundingUnavailable,
}

impl AcceptedChatTurnCheckpointKind {
    fn localization_key(self) -> &'static str {
        match self {
            Self::WebGroundingUnavailable => AMBIENT_SEARCH_UNAVAILABLE_KEY,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::WebGroundingUnavailable => "web_grounding_unavailable",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordAcceptedChatTurnCheckpointRequest {
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub kind: AcceptedChatTurnCheckpointKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedChatTurnCheckpointReceipt {
    pub message_id: i64,
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub kind: AcceptedChatTurnCheckpointKind,
    pub localization_key: &'static str,
    pub recorded_at_ms: i64,
    pub created: bool,
}

fn required_checkpoint_id(name: &str, value: &str) -> rusqlite::Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "accepted turn checkpoint requires a valid {name}"
        )));
    }
    Ok(value.to_string())
}

impl PersistenceEngine {
    fn record_accepted_chat_turn_checkpoint(
        &self,
        request: RecordAcceptedChatTurnCheckpointRequest,
    ) -> rusqlite::Result<AcceptedChatTurnCheckpointReceipt> {
        let session_id = required_checkpoint_id("session_id", &request.session_id)?;
        let turn_id = required_checkpoint_id("turn_id", &request.turn_id)?;
        let generation_token =
            required_checkpoint_id("generation_token", &request.generation_token)?;
        let kind = request.kind;
        let localization_key = kind.localization_key();
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &session_id, &self.workspace_id)?;
        let turn = transaction
            .query_row(
                "SELECT turns.agent_id, turns.provider_id, turns.model_id
                 FROM chat_turns turns
                 WHERE turns.workspace_id = ?1 AND turns.session_id = ?2
                   AND turns.turn_id = ?3 AND turns.generation_token = ?4
                   AND turns.status = 'running'
                   AND EXISTS (
                     SELECT 1 FROM chat_messages messages
                     WHERE messages.workspace_id = turns.workspace_id
                       AND messages.session_id = turns.session_id
                       AND messages.role = 'user'
                       AND json_extract(messages.metadata_json, '$.turnId') = turns.turn_id
                       AND json_extract(messages.metadata_json, '$.generationToken') = turns.generation_token
                       AND json_extract(messages.metadata_json, '$.turnState') = 'accepted'
                   )",
                params![workspace_id, session_id, turn_id, generation_token],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((agent_id, provider_id, model_id)) = turn else {
            return Err(rusqlite::Error::InvalidParameterName(
                "accepted turn checkpoint does not match one pending accepted turn".to_string(),
            ));
        };
        let existing = transaction
            .query_row(
                "SELECT id, timestamp_ms FROM chat_messages
                 WHERE workspace_id = ?1 AND session_id = ?2 AND role = 'system'
                   AND json_extract(metadata_json, '$.checkpointForTurnId') = ?3
                   AND json_extract(metadata_json, '$.generationToken') = ?4
                   AND json_extract(metadata_json, '$.checkpointKind') = ?5
                 ORDER BY id ASC LIMIT 1",
                params![
                    workspace_id,
                    session_id,
                    turn_id,
                    generation_token,
                    kind.as_str()
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        if let Some((message_id, recorded_at_ms)) = existing {
            transaction.commit()?;
            return Ok(AcceptedChatTurnCheckpointReceipt {
                message_id,
                session_id,
                turn_id,
                generation_token,
                kind,
                localization_key,
                recorded_at_ms,
                created: false,
            });
        }
        let recorded_at_ms = unix_time_ms();
        let metadata = json!({
            "checkpointKind": kind.as_str(),
            "checkpointForTurnId": turn_id,
            "generationToken": generation_token,
            "sessionId": session_id,
            "localizationKey": localization_key,
            "turnState": "accepted",
            "uiOnlyCheckpoint": true,
        });
        transaction.execute(
            "INSERT INTO chat_messages (
                workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms
             ) VALUES (?1, ?2, ?3, 'system', ?4, ?5, ?6, ?7, 0, 'raw', ?8)",
            params![
                workspace_id,
                session_id,
                agent_id,
                localization_key,
                provider_id,
                model_id,
                metadata.to_string(),
                recorded_at_ms,
            ],
        )?;
        let message_id = transaction.last_insert_rowid();
        transaction.commit()?;
        Ok(AcceptedChatTurnCheckpointReceipt {
            message_id,
            session_id,
            turn_id,
            generation_token,
            kind,
            localization_key,
            recorded_at_ms,
            created: true,
        })
    }
}

#[tauri::command]
pub async fn record_accepted_chat_turn_checkpoint(
    request: RecordAcceptedChatTurnCheckpointRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<AcceptedChatTurnCheckpointReceipt, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.record_accepted_chat_turn_checkpoint(request)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambient_checkpoint_is_accepted_turn_bound_idempotent_and_ui_only() {
        let database_path = std::env::temp_dir().join(format!(
            "oomu-accepted-turn-checkpoint-{}-{}.sqlite",
            std::process::id(),
            unix_time_ms()
        ));
        let engine = PersistenceEngine::initialize_at(database_path.clone())
            .expect("checkpoint database initializes");
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-checkpoint".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-checkpoint".to_string(),
                title: Some("Ambient checkpoint".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .expect("checkpoint session initializes");
        engine
            .accept_chat_turn(AcceptChatTurnRequest {
                turn_id: "turn-checkpoint".to_string(),
                generation_token: "generation-checkpoint".to_string(),
                parent_turn_id: None,
                root_turn_id: "turn-checkpoint".to_string(),
                turn_kind: "root".to_string(),
                session_id: session.id.clone(),
                agent_id: session.agent_id.clone(),
                provider_id: session.provider_id.clone(),
                model_id: session.model_id.clone(),
                message: "What changed today?".to_string(),
            })
            .expect("user turn is durably accepted");
        let request = RecordAcceptedChatTurnCheckpointRequest {
            session_id: session.id.clone(),
            turn_id: "turn-checkpoint".to_string(),
            generation_token: "generation-checkpoint".to_string(),
            kind: AcceptedChatTurnCheckpointKind::WebGroundingUnavailable,
        };

        let first = engine
            .record_accepted_chat_turn_checkpoint(request.clone())
            .expect("first checkpoint is recorded");
        let repeated = engine
            .record_accepted_chat_turn_checkpoint(request)
            .expect("checkpoint replay is idempotent");
        assert!(first.created);
        assert!(!repeated.created);
        assert_eq!(first.message_id, repeated.message_id);
        assert_eq!(first.localization_key, AMBIENT_SEARCH_UNAVAILABLE_KEY);
        assert_eq!(engine.select_chat_messages(&session.id).unwrap().len(), 2);
        assert_eq!(engine.get_chat_history(&session.id, 20).unwrap().len(), 1);

        let invalid = RecordAcceptedChatTurnCheckpointRequest {
            session_id: session.id,
            turn_id: "turn-checkpoint".to_string(),
            generation_token: "wrong-generation".to_string(),
            kind: AcceptedChatTurnCheckpointKind::WebGroundingUnavailable,
        };
        assert!(engine
            .record_accepted_chat_turn_checkpoint(invalid)
            .is_err());
        drop(engine);
        let _ = std::fs::remove_file(database_path);
    }
}
