use super::{AcceptChatTurnRequest, AcceptedChatTurn};
use crate::db::{
    validate_chat_turn_context_fields, workspace_id_for_chat_session, ChatTurnPersistenceContext,
    PersistenceEngine,
};
use rusqlite::{params, OptionalExtension};

const INTERRUPTED_TURN_RESUME_MAX_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const CLOCK_SKEW_ALLOWANCE_MS: i64 = 5 * 60 * 1_000;

struct InterruptedTurn {
    message_id: i64,
    message: String,
    session_was_empty_before_acceptance: bool,
    interrupted_at_ms: i64,
    restore_dynamic_session_binding: bool,
}

fn resumed_turn_route<'a>(
    context: &'a ChatTurnPersistenceContext,
    restore_dynamic_session_binding: bool,
) -> (&'a str, &'a str) {
    if restore_dynamic_session_binding {
        ("dynamic", "dynamic")
    } else {
        (context.provider_id.as_str(), context.model_id.as_str())
    }
}

fn validate_interrupted_turn(
    interrupted: &InterruptedTurn,
    original_message: &str,
    now: i64,
) -> rusqlite::Result<()> {
    if interrupted.message != original_message
        || interrupted.interrupted_at_ms > now.saturating_add(CLOCK_SKEW_ALLOWANCE_MS)
        || now.saturating_sub(interrupted.interrupted_at_ms) > INTERRUPTED_TURN_RESUME_MAX_AGE_MS
    {
        return Err(resume_rejected());
    }
    Ok(())
}

fn restore_dynamic_session_binding(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    context: &ChatTurnPersistenceContext,
    now: i64,
    required: bool,
) -> rusqlite::Result<()> {
    if !required {
        return Ok(());
    }
    let changed = transaction.execute(
        "UPDATE chat_sessions
         SET provider_id = 'dynamic', model_id = 'dynamic',
             dynamic_routing_override = 1, updated_at_ms = ?4
         WHERE id = ?1 AND workspace_id = ?2 AND agent_id = ?3",
        params![context.session_id, workspace_id, context.agent_id, now],
    )?;
    (changed == 1).then_some(()).ok_or_else(resume_rejected)
}

struct AcceptedTurnInterruptState {
    status: String,
    claimed_at_ms: Option<i64>,
    completed_at_ms: Option<i64>,
    message_id: i64,
    turn_state: Option<String>,
    has_terminal: bool,
}

impl AcceptedTurnInterruptState {
    fn already_interrupted(&self) -> bool {
        self.status == "failed"
            && self.claimed_at_ms.is_none()
            && self.completed_at_ms.is_some()
            && self.turn_state.as_deref() == Some("interrupted")
            && !self.has_terminal
    }

    fn can_interrupt(&self) -> bool {
        self.status == "running" && self.completed_at_ms.is_none() && !self.has_terminal
    }
}

fn accepted_turn_interrupt_state(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    context: &ChatTurnPersistenceContext,
) -> rusqlite::Result<Option<AcceptedTurnInterruptState>> {
    transaction
        .query_row(
            "SELECT turns.status, turns.response_claimed_at_ms, turns.completed_at_ms,
                    message.id, json_extract(message.metadata_json, '$.turnState'),
                    EXISTS(
                        SELECT 1 FROM chat_messages terminal
                        WHERE terminal.workspace_id=turns.workspace_id
                          AND terminal.session_id=turns.session_id
                          AND json_extract(terminal.metadata_json,
                              '$.terminalResultForTurnId')=turns.turn_id
                    )
             FROM chat_turns turns
             JOIN chat_messages message
               ON message.workspace_id=turns.workspace_id
              AND message.session_id=turns.session_id
              AND message.agent_id=turns.agent_id
              AND message.role='user'
              AND json_extract(message.metadata_json,'$.turnId')=turns.turn_id
              AND json_extract(message.metadata_json,'$.generationToken')=turns.generation_token
             WHERE turns.turn_id=?1 AND turns.generation_token=?2
               AND turns.workspace_id=?3 AND turns.session_id=?4 AND turns.agent_id=?5
               AND turns.provider_id=?6 AND turns.model_id=?7
               AND turns.root_turn_id=?8 AND turns.turn_kind=?9
               AND COALESCE(turns.parent_turn_id,'')=COALESCE(?10,'')",
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
                Ok(AcceptedTurnInterruptState {
                    status: row.get(0)?,
                    claimed_at_ms: row.get(1)?,
                    completed_at_ms: row.get(2)?,
                    message_id: row.get(3)?,
                    turn_state: row.get(4)?,
                    has_terminal: row.get(5)?,
                })
            },
        )
        .optional()
}

impl PersistenceEngine {
    pub(crate) fn interrupt_accepted_chat_turn(
        &self,
        context: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<Option<i64>> {
        validate_chat_turn_context_fields(context)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &context.session_id, &self.workspace_id)?;
        let Some(state) = accepted_turn_interrupt_state(&transaction, &workspace_id, context)?
        else {
            transaction.commit()?;
            return Ok(None);
        };
        if state.already_interrupted() {
            transaction.commit()?;
            return Ok(Some(state.message_id));
        }
        if !state.can_interrupt() {
            transaction.commit()?;
            return Ok(None);
        }
        let interrupted_at_ms = crate::foundation::clock::unix_time_ms_i64();
        let turn_changed = transaction.execute(
            "UPDATE chat_turns SET status='failed',completed_at_ms=?10,
                 response_claimed_at_ms=NULL
             WHERE turn_id=?1 AND generation_token=?2 AND workspace_id=?3
               AND session_id=?4 AND agent_id=?5 AND provider_id=?6 AND model_id=?7
               AND root_turn_id=?8 AND turn_kind=?9
               AND COALESCE(parent_turn_id,'')=COALESCE(?11,'')
               AND status='running' AND completed_at_ms IS NULL
               AND NOT EXISTS (
                   SELECT 1 FROM chat_messages terminal
                   WHERE terminal.workspace_id=chat_turns.workspace_id
                     AND terminal.session_id=chat_turns.session_id
                     AND json_extract(terminal.metadata_json,
                         '$.terminalResultForTurnId')=chat_turns.turn_id
               )",
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
                interrupted_at_ms,
                context.parent_turn_id,
            ],
        )?;
        let message_changed = transaction.execute(
            "UPDATE chat_messages
             SET metadata_json=json_set(COALESCE(metadata_json,'{}'),'$.turnState','interrupted')
             WHERE id=?1 AND workspace_id=?2 AND session_id=?3 AND agent_id=?4
               AND role='user' AND json_extract(metadata_json,'$.turnId')=?5
               AND json_extract(metadata_json,'$.generationToken')=?6",
            params![
                state.message_id,
                workspace_id,
                context.session_id,
                context.agent_id,
                context.turn_id,
                context.generation_token,
            ],
        )?;
        if turn_changed != 1 || message_changed != 1 {
            return Err(resume_rejected());
        }
        transaction.commit()?;
        Ok(Some(state.message_id))
    }

    pub fn resume_interrupted_chat_turn(
        &self,
        request: AcceptChatTurnRequest,
    ) -> rusqlite::Result<AcceptedChatTurn> {
        if request.message.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "interrupted chat turn resume requires its original user message".to_string(),
            ));
        }
        let context = request.persistence_context();
        validate_chat_turn_context_fields(&context)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &context.session_id, &self.workspace_id)?;
        let interrupted = transaction
            .query_row(
                "SELECT message.id, message.content,
                        COALESCE(json_extract(message.metadata_json,
                            '$.sessionWasEmptyBeforeAcceptance'), 0),
                        turns.completed_at_ms,
                        ((lower(sessions.provider_id) = 'dynamic'
                          AND lower(sessions.model_id) = 'dynamic')
                         OR COALESCE(sessions.dynamic_routing_override, 0) = 1)
                 FROM chat_turns turns
                 JOIN chat_sessions sessions
                   ON sessions.id = turns.session_id
                  AND sessions.workspace_id = turns.workspace_id
                  AND sessions.agent_id = turns.agent_id
                 JOIN chat_messages message
                   ON message.workspace_id = turns.workspace_id
                  AND message.session_id = turns.session_id
                  AND message.agent_id = turns.agent_id
                  AND message.role = 'user'
                  AND json_extract(message.metadata_json, '$.turnId') = turns.turn_id
                 WHERE turns.turn_id = ?1 AND turns.generation_token = ?2
                   AND turns.workspace_id = ?3 AND turns.session_id = ?4
                   AND turns.agent_id = ?5 AND turns.provider_id = ?6 AND turns.model_id = ?7
                   AND turns.root_turn_id = ?8 AND turns.turn_kind = ?9
                   AND COALESCE(turns.parent_turn_id, '') = COALESCE(?10, '')
                   AND turns.status = 'failed' AND turns.completed_at_ms IS NOT NULL
                   AND turns.response_claimed_at_ms IS NULL
                   AND json_extract(message.metadata_json, '$.generationToken') = ?2
                   AND json_extract(message.metadata_json, '$.turnState') = 'interrupted'
                   AND (SELECT COUNT(*) FROM chat_messages duplicate
                        WHERE duplicate.workspace_id = turns.workspace_id
                          AND duplicate.session_id = turns.session_id
                          AND duplicate.role = 'user'
                          AND json_extract(duplicate.metadata_json, '$.turnId') = turns.turn_id) = 1
                   AND NOT EXISTS (
                        SELECT 1 FROM chat_messages terminal
                        WHERE terminal.workspace_id = turns.workspace_id
                          AND terminal.session_id = turns.session_id
                          AND json_extract(terminal.metadata_json,
                              '$.terminalResultForTurnId') = turns.turn_id
                   )",
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
                    Ok(InterruptedTurn {
                        message_id: row.get(0)?,
                        message: row.get(1)?,
                        session_was_empty_before_acceptance: row.get(2)?,
                        interrupted_at_ms: row.get(3)?,
                        restore_dynamic_session_binding: row.get(4)?,
                    })
                },
            )
            .optional()?;
        let Some(interrupted) = interrupted else {
            return Err(resume_rejected());
        };
        let now = crate::foundation::clock::unix_time_ms_i64();
        validate_interrupted_turn(&interrupted, &request.message, now)?;
        let (resumed_provider_id, resumed_model_id) =
            resumed_turn_route(&context, interrupted.restore_dynamic_session_binding);
        let turn_changed = transaction.execute(
            "UPDATE chat_turns SET status = 'running', completed_at_ms = NULL,
                 provider_id = ?12, model_id = ?13
             WHERE turn_id = ?1 AND generation_token = ?2 AND workspace_id = ?3
               AND session_id = ?4 AND agent_id = ?5 AND provider_id = ?6 AND model_id = ?7
               AND root_turn_id = ?8 AND turn_kind = ?9
               AND COALESCE(parent_turn_id, '') = COALESCE(?10, '')
               AND status = 'failed' AND completed_at_ms = ?11
               AND response_claimed_at_ms IS NULL",
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
                interrupted.interrupted_at_ms,
                resumed_provider_id,
                resumed_model_id,
            ],
        )?;
        let message_changed = transaction.execute(
            "UPDATE chat_messages
             SET metadata_json = json_set(COALESCE(metadata_json, '{}'),
                 '$.turnState', 'accepted')
             WHERE id = ?1 AND workspace_id = ?2 AND session_id = ?3 AND agent_id = ?4
               AND role = 'user' AND content = ?5
               AND json_extract(metadata_json, '$.turnId') = ?6
               AND json_extract(metadata_json, '$.generationToken') = ?7
               AND json_extract(metadata_json, '$.turnState') = 'interrupted'",
            params![
                interrupted.message_id,
                workspace_id,
                context.session_id,
                context.agent_id,
                request.message,
                context.turn_id,
                context.generation_token,
            ],
        )?;
        if turn_changed != 1 || message_changed != 1 {
            return Err(resume_rejected());
        }
        restore_dynamic_session_binding(
            &transaction,
            &workspace_id,
            &context,
            now,
            interrupted.restore_dynamic_session_binding,
        )?;
        transaction.commit()?;
        Ok(AcceptedChatTurn {
            turn_id: context.turn_id,
            message_id: interrupted.message_id,
            accepted: true,
            session_was_empty_before_acceptance: interrupted.session_was_empty_before_acceptance,
        })
    }
}

fn resume_rejected() -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(
        "interrupted chat turn does not match its recoverable immutable state".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        AbandonAcceptedChatTurnRequest, CreateChatSessionRequest, FinalizeAcceptedChatTurnRequest,
    };

    fn fixture(
        name: &str,
    ) -> (
        std::path::PathBuf,
        PersistenceEngine,
        crate::db::ChatSessionRecord,
    ) {
        let root = std::env::temp_dir().join(format!(
            "oomu-chat-resume-{name}-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: format!("agent-{name}"),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some(format!("Resume {name}")),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            })
            .unwrap();
        (root, engine, session)
    }

    fn request(session: &crate::db::ChatSessionRecord, name: &str) -> AcceptChatTurnRequest {
        AcceptChatTurnRequest {
            turn_id: format!("turn-{name}"),
            generation_token: format!("generation-{name}"),
            parent_turn_id: None,
            root_turn_id: format!("turn-{name}"),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            message: format!("Keep the exact {name} request."),
        }
    }

    fn interrupt(engine: &PersistenceEngine, request: &AcceptChatTurnRequest) -> i64 {
        let accepted = engine.accept_chat_turn(request.clone()).unwrap();
        engine.mark_interrupted_actions().unwrap();
        accepted.message_id
    }

    #[test]
    fn restart_resume_atomically_reuses_the_exact_interrupted_turn_once() {
        let (root, engine, session) = fixture("exact");
        let request = request(&session, "exact");
        let message_id = interrupt(&engine, &request);

        let resumed = engine
            .resume_interrupted_chat_turn(request.clone())
            .unwrap();
        assert_eq!(resumed.message_id, message_id);
        assert!(resumed.accepted);
        assert!(engine.resume_interrupted_chat_turn(request).is_err());

        let connection = engine.open_connection().unwrap();
        let (status, completed, claimed): (String, Option<i64>, Option<i64>) = connection
            .query_row(
                "SELECT status, completed_at_ms, response_claimed_at_ms
                 FROM chat_turns WHERE turn_id = 'turn-exact'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (status.as_str(), completed, claimed),
            ("running", None, None)
        );
        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0]
            .metadata_json
            .as_deref()
            .unwrap()
            .contains("\"turnState\":\"accepted\""));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restart_resume_rejects_foreign_changed_stale_and_terminal_turns() {
        let (root, engine, session) = fixture("guards");
        let original = request(&session, "guards");
        interrupt(&engine, &original);

        let mut changed = original.clone();
        changed.model_id = "different-model".to_string();
        assert!(engine.resume_interrupted_chat_turn(changed).is_err());
        let mut foreign = original.clone();
        foreign.agent_id = "different-agent".to_string();
        assert!(engine.resume_interrupted_chat_turn(foreign).is_err());
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE chat_turns SET completed_at_ms = ?1 WHERE turn_id = ?2",
                params![
                    crate::foundation::clock::unix_time_ms_i64()
                        - INTERRUPTED_TURN_RESUME_MAX_AGE_MS
                        - 1,
                    original.turn_id,
                ],
            )
            .unwrap();
        assert!(engine
            .resume_interrupted_chat_turn(original.clone())
            .is_err());
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE chat_turns SET completed_at_ms = ?1 WHERE turn_id = ?2",
                params![
                    crate::foundation::clock::unix_time_ms_i64(),
                    original.turn_id,
                ],
            )
            .unwrap();
        engine
            .finalize_accepted_chat_turn(FinalizeAcceptedChatTurnRequest {
                turn_id: original.turn_id.clone(),
                generation_token: original.generation_token.clone(),
                parent_turn_id: None,
                root_turn_id: original.root_turn_id.clone(),
                turn_kind: original.turn_kind.clone(),
                session_id: original.session_id.clone(),
                agent_id: original.agent_id.clone(),
                provider_id: original.provider_id.clone(),
                model_id: original.model_id.clone(),
                role: "system".to_string(),
                content: "This interrupted turn ended safely.".to_string(),
                status: "failed".to_string(),
            })
            .unwrap();
        assert!(engine.resume_interrupted_chat_turn(original).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restart_resume_preserves_response_claim_and_deletion_guards() {
        let (claimed_root, claimed_engine, claimed_session) = fixture("claimed");
        let claimed = request(&claimed_session, "claimed");
        claimed_engine.accept_chat_turn(claimed.clone()).unwrap();
        claimed_engine
            .begin_or_claim_chat_turn_response(&claimed.persistence_context())
            .unwrap();
        claimed_engine.mark_interrupted_actions().unwrap();
        assert!(claimed_engine
            .resume_interrupted_chat_turn(claimed)
            .is_err());
        std::fs::remove_dir_all(claimed_root).unwrap();

        let (deleted_root, deleted_engine, deleted_session) = fixture("deleted");
        let deleted = request(&deleted_session, "deleted");
        interrupt(&deleted_engine, &deleted);
        assert!(deleted_engine
            .stage_chat_session_deletion_by_id(&deleted_session.id)
            .unwrap());
        assert!(deleted_engine
            .resume_interrupted_chat_turn(deleted)
            .is_err());
        std::fs::remove_dir_all(deleted_root).unwrap();
    }

    #[test]
    fn sprint_304_user_stop_releases_the_exact_claim_for_visible_recovery() {
        let (root, engine, session) = fixture("user-stop");
        let request = request(&session, "user-stop");
        let accepted = engine.accept_chat_turn(request.clone()).unwrap();
        let context = request.persistence_context();
        engine.begin_or_claim_chat_turn_response(&context).unwrap();

        assert_eq!(
            engine.interrupt_accepted_chat_turn(&context).unwrap(),
            Some(accepted.message_id)
        );
        assert_eq!(
            engine
                .abandon_accepted_chat_turn(AbandonAcceptedChatTurnRequest {
                    turn_id: context.turn_id.clone(),
                    generation_token: context.generation_token.clone(),
                    parent_turn_id: context.parent_turn_id.clone(),
                    root_turn_id: context.root_turn_id.clone(),
                    turn_kind: context.turn_kind.clone(),
                    session_id: context.session_id.clone(),
                    agent_id: context.agent_id.clone(),
                    provider_id: context.provider_id.clone(),
                    model_id: context.model_id.clone(),
                    content: "This must not replace recoverable work.".to_string(),
                })
                .unwrap(),
            None
        );
        let resumed = engine.resume_interrupted_chat_turn(request).unwrap();
        assert_eq!(resumed.message_id, accepted.message_id);

        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0]
            .metadata_json
            .as_deref()
            .unwrap()
            .contains("\"turnState\":\"accepted\""));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sprint_304_dynamic_turn_replays_the_exact_claimed_local_route() {
        let (root, engine, session) = fixture("dynamic-local-stop");
        let dynamic_request = request(&session, "dynamic-local-stop");
        let accepted = engine.accept_chat_turn(dynamic_request.clone()).unwrap();
        let local_context = ChatTurnPersistenceContext {
            provider_id: "prov-local-sprint-302".to_string(),
            model_id: "gemma-4-E4B-it-qat-q4_0-gguf".to_string(),
            ..dynamic_request.persistence_context()
        };

        engine
            .begin_or_claim_chat_turn_response(&local_context)
            .unwrap();
        assert_eq!(
            engine.interrupt_accepted_chat_turn(&local_context).unwrap(),
            Some(accepted.message_id)
        );
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE chat_sessions SET provider_id = ?2, model_id = ?3 WHERE id = ?1",
                params![
                    session.id,
                    local_context.provider_id,
                    local_context.model_id,
                ],
            )
            .unwrap();

        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].provider_id.as_deref(),
            Some(local_context.provider_id.as_str())
        );
        assert_eq!(
            messages[0].model_id.as_deref(),
            Some(local_context.model_id.as_str())
        );

        let resumed = engine
            .resume_interrupted_chat_turn(AcceptChatTurnRequest {
                provider_id: local_context.provider_id.clone(),
                model_id: local_context.model_id.clone(),
                ..dynamic_request.clone()
            })
            .unwrap();
        assert_eq!(resumed.message_id, accepted.message_id);
        let restored_route: (String, String) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT provider_id, model_id FROM chat_turns WHERE turn_id = ?1",
                params![dynamic_request.turn_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            restored_route,
            ("dynamic".to_string(), "dynamic".to_string())
        );
        let restored_session_route: (String, String, Option<bool>) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT provider_id, model_id, dynamic_routing_override
                 FROM chat_sessions WHERE id = ?1",
                params![session.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            restored_session_route,
            ("dynamic".to_string(), "dynamic".to_string(), Some(true))
        );
        engine.accept_chat_turn(dynamic_request).unwrap();
        engine
            .begin_or_claim_chat_turn_response(&local_context)
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
