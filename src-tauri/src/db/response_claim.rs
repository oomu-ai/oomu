use super::{workspace_id_for_chat_session, ChatTurnPersistenceContext, PersistenceEngine};
use crate::foundation::clock::unix_time_ms_i64 as unix_time_ms;
use rusqlite::{params, OptionalExtension};

const CHAT_TURN_RESPONSE_CLAIM_CONFLICT: &str = "chat_turn_response_claim_conflict";
const CHAT_TURN_RESPONSE_CLAIM_MISMATCH: &str = "chat_turn_response_claim_mismatch";
pub(crate) const AUTO_TURN_KIND: &str = "auto_turn";

pub(super) fn validate_chat_turn_context_fields(
    context: &ChatTurnPersistenceContext,
) -> rusqlite::Result<()> {
    let required = [
        ("turn_id", context.turn_id.as_str()),
        ("generation_token", context.generation_token.as_str()),
        ("session_id", context.session_id.as_str()),
        ("agent_id", context.agent_id.as_str()),
        ("provider_id", context.provider_id.as_str()),
        ("model_id", context.model_id.as_str()),
        ("root_turn_id", context.root_turn_id.as_str()),
        ("turn_kind", context.turn_kind.as_str()),
    ];
    if let Some((field, _)) = required.iter().find(|(_, value)| value.trim().is_empty()) {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "chat turn {field} must not be empty"
        )));
    }
    if required.iter().any(|(_, value)| value.len() > 512)
        || context
            .parent_turn_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 512)
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "chat turn context fields must be 512 characters or fewer".to_string(),
        ));
    }

    match context.turn_kind.as_str() {
        "root" => {
            if context.parent_turn_id.is_some() || context.root_turn_id != context.turn_id {
                return Err(rusqlite::Error::InvalidParameterName(
                    "root chat turns must have no parent and must identify themselves as the root"
                        .to_string(),
                ));
            }
        }
        "queued" | "steer" | "retry" | AUTO_TURN_KIND => {
            if context.parent_turn_id.is_none() {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "{} chat turns require a parent turn",
                    context.turn_kind
                )));
            }
        }
        _ => {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "unsupported chat turn kind: {}",
                context.turn_kind
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_chat_turn_parent(
    connection: &rusqlite::Connection,
    context: &ChatTurnPersistenceContext,
) -> rusqlite::Result<()> {
    let Some(parent_turn_id) = context.parent_turn_id.as_deref() else {
        return Ok(());
    };
    let parent = connection
        .query_row(
            "SELECT turns.session_id, turns.agent_id, turns.root_turn_id,
                    turns.provider_id, turns.model_id, turns.response_claimed_at_ms,
                    COALESCE(sessions.dynamic_routing_override, 0)
             FROM chat_turns turns
             JOIN chat_sessions sessions
               ON sessions.id = turns.session_id
              AND sessions.workspace_id = turns.workspace_id
              AND sessions.agent_id = turns.agent_id
             WHERE turns.turn_id = ?1",
            params![parent_turn_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, bool>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((
        parent_session_id,
        parent_agent_id,
        parent_root_turn_id,
        parent_provider_id,
        parent_model_id,
        parent_response_claimed_at_ms,
        session_dynamic_routing_override,
    )) = parent
    else {
        return Err(rusqlite::Error::InvalidParameterName(
            "derived chat turn parent does not exist".to_string(),
        ));
    };
    let route_matches =
        parent_provider_id == context.provider_id && parent_model_id == context.model_id;
    let dynamic_child_follows_claimed_parent = session_dynamic_routing_override
        && parent_response_claimed_at_ms.is_some()
        && context.provider_id.eq_ignore_ascii_case("dynamic")
        && context.model_id.eq_ignore_ascii_case("dynamic");
    if parent_session_id != context.session_id
        || parent_agent_id != context.agent_id
        || parent_root_turn_id != context.root_turn_id
        || (!route_matches && !dynamic_child_follows_claimed_parent)
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "derived chat turn ancestry crosses a session, agent, root, provider, or model boundary"
                .to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn is_chat_turn_response_claim_conflict(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::InvalidParameterName(value)
            if value == CHAT_TURN_RESPONSE_CLAIM_CONFLICT
    )
}

#[cfg(test)]
pub(crate) fn is_chat_turn_response_claim_mismatch(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::InvalidParameterName(value)
            if value == CHAT_TURN_RESPONSE_CLAIM_MISMATCH
    )
}

fn rebind_claimed_user_message_route(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    context: &ChatTurnPersistenceContext,
) -> rusqlite::Result<()> {
    let changed = transaction.execute(
        "UPDATE chat_messages
         SET provider_id = ?5, model_id = ?6
         WHERE workspace_id = ?1 AND session_id = ?2 AND agent_id = ?3
           AND role = 'user'
           AND json_extract(metadata_json, '$.turnId') = ?4
           AND json_extract(metadata_json, '$.generationToken') = ?7
           AND json_extract(metadata_json, '$.turnState') = 'accepted'",
        params![
            workspace_id,
            context.session_id,
            context.agent_id,
            context.turn_id,
            context.provider_id,
            context.model_id,
            context.generation_token,
        ],
    )?;
    if changed == 1 {
        return Ok(());
    }
    let mismatched_user_message: bool = transaction.query_row(
        "SELECT EXISTS (
             SELECT 1
             FROM chat_messages
             WHERE workspace_id = ?1 AND session_id = ?2 AND agent_id = ?3
               AND role = 'user'
               AND json_extract(metadata_json, '$.turnId') = ?4
         )",
        params![
            workspace_id,
            context.session_id,
            context.agent_id,
            context.turn_id,
        ],
        |row| row.get(0),
    )?;
    if mismatched_user_message {
        Err(rusqlite::Error::InvalidParameterName(
            CHAT_TURN_RESPONSE_CLAIM_MISMATCH.to_string(),
        ))
    } else {
        // Native actions and derived background turns have a durable turn row
        // but no renderer-authored user message to rebind.
        Ok(())
    }
}

fn existing_response_claim_matches(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    context: &ChatTurnPersistenceContext,
) -> rusqlite::Result<bool> {
    transaction.query_row(
        "
        SELECT EXISTS (
            SELECT 1
            FROM chat_turns turns
            JOIN chat_sessions sessions
              ON sessions.id = turns.session_id
             AND sessions.workspace_id = turns.workspace_id
            WHERE turns.turn_id = ?1
              AND turns.generation_token = ?2
              AND turns.workspace_id = ?3
              AND turns.session_id = ?4
              AND turns.agent_id = ?5
              AND turns.root_turn_id = ?6
              AND turns.turn_kind = ?7
              AND COALESCE(turns.parent_turn_id, '') = COALESCE(?8, '')
              AND turns.provider_id = ?9
              AND turns.model_id = ?10
              AND turns.response_claimed_at_ms IS NOT NULL
              AND (
                    turns.status = 'running'
                 OR (
                        turns.status IN ('completed', 'failed', 'cancelled', 'escalated')
                    AND EXISTS (
                        SELECT 1
                        FROM chat_messages messages
                        WHERE messages.workspace_id = turns.workspace_id
                          AND messages.session_id = turns.session_id
                          AND json_extract(messages.metadata_json, '$.terminalResultForTurnId') = turns.turn_id
                    )
                 )
              )
              AND sessions.agent_id = turns.agent_id
        )
        ",
        params![
            context.turn_id,
            context.generation_token,
            workspace_id,
            context.session_id,
            context.agent_id,
            context.root_turn_id,
            context.turn_kind,
            context.parent_turn_id,
            context.provider_id,
            context.model_id,
        ],
        |row| row.get(0),
    )
}

impl PersistenceEngine {
    pub(crate) fn release_chat_turn_response_claim(
        &self,
        context: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<bool> {
        validate_chat_turn_context_fields(context)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &context.session_id, &self.workspace_id)?;
        let changed = transaction.execute(
            "UPDATE chat_turns
             SET response_claimed_at_ms = NULL
             WHERE turn_id = ?1 AND generation_token = ?2
               AND workspace_id = ?3 AND session_id = ?4 AND agent_id = ?5
               AND provider_id = ?6 AND model_id = ?7
               AND root_turn_id = ?8 AND turn_kind = ?9
               AND COALESCE(parent_turn_id, '') = COALESCE(?10, '')
               AND status = 'running' AND response_claimed_at_ms IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM chat_messages terminal
                   WHERE terminal.workspace_id = chat_turns.workspace_id
                     AND terminal.session_id = chat_turns.session_id
                     AND json_extract(terminal.metadata_json, '$.terminalResultForTurnId') = chat_turns.turn_id
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
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn ensure_chat_turn_for_native_action(
        &self,
        context: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<()> {
        validate_chat_turn_context_fields(context)?;
        let validate_existing = || -> rusqlite::Result<bool> {
            let connection = self.open_connection()?;
            let matches: i64 = connection.query_row(
                "
                SELECT COUNT(*)
                FROM chat_turns turns
                JOIN chat_sessions sessions
                  ON sessions.id = turns.session_id
                 AND sessions.workspace_id = turns.workspace_id
                WHERE turns.turn_id = ?1
                  AND turns.generation_token = ?2
                  AND turns.session_id = ?3
                  AND turns.agent_id = ?4
                  AND (
                        (turns.provider_id = ?5 AND turns.model_id = ?6)
                     OR (
                            lower(?5) = 'dynamic'
                        AND lower(?6) = 'dynamic'
                        AND turns.response_claimed_at_ms IS NOT NULL
                        AND (
                              (
                                  lower(sessions.provider_id) = 'dynamic'
                                  AND lower(sessions.model_id) = 'dynamic'
                              )
                              OR COALESCE(sessions.dynamic_routing_override, 0) = 1
                        )
                     )
                  )
                  AND turns.root_turn_id = ?7
                  AND turns.turn_kind = ?8
                  AND COALESCE(turns.parent_turn_id, '') = COALESCE(?9, '')
                  AND (
                        (turns.status = 'running' AND turns.response_claimed_at_ms IS NULL)
                     OR (
                            turns.status IN ('completed', 'escalated')
                        AND turns.response_claimed_at_ms IS NOT NULL
                     )
                  )
                  AND sessions.agent_id = turns.agent_id
                ",
                params![
                    context.turn_id,
                    context.generation_token,
                    context.session_id,
                    context.agent_id,
                    context.provider_id,
                    context.model_id,
                    context.root_turn_id,
                    context.turn_kind,
                    context.parent_turn_id,
                ],
                |row| row.get(0),
            )?;
            Ok(matches == 1)
        };
        if validate_existing()? {
            return Ok(());
        }
        if self.select_chat_turn_context(&context.turn_id)?.is_some() {
            return Err(rusqlite::Error::InvalidParameterName(
                "native action chat turn does not match an active immutable context".to_string(),
            ));
        }
        match self.begin_chat_turn(context) {
            Ok(()) => Ok(()),
            Err(_insert_error) if validate_existing()? => Ok(()),
            Err(insert_error) => Err(insert_error),
        }
    }

    pub fn begin_or_claim_chat_turn_response(
        &self,
        context: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<()> {
        match self.begin_chat_turn(context) {
            Ok(()) => self.claim_prebound_chat_turn_response(context),
            Err(insert_error)
                if insert_error.sqlite_error_code()
                    == Some(rusqlite::ErrorCode::ConstraintViolation) =>
            {
                self.claim_prebound_chat_turn_response(context)
            }
            Err(insert_error) => Err(insert_error),
        }
    }

    fn claim_prebound_chat_turn_response(
        &self,
        context: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<()> {
        validate_chat_turn_context_fields(context)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &context.session_id, &self.workspace_id)?;
        let claimed_at_ms = unix_time_ms();
        let changed = transaction.execute(
            "
            UPDATE chat_turns
            SET provider_id = ?5,
                model_id = ?6,
                response_claimed_at_ms = ?10
            WHERE turn_id = ?1
              AND generation_token = ?2
              AND workspace_id = ?3
              AND session_id = ?4
              AND agent_id = ?7
              AND root_turn_id = ?8
              AND turn_kind = ?9
              AND COALESCE(parent_turn_id, '') = COALESCE(?11, '')
              AND status = 'running'
              AND response_claimed_at_ms IS NULL
              AND (
                    (provider_id = ?5 AND model_id = ?6)
                 OR (
                        lower(provider_id) = 'dynamic'
                    AND lower(model_id) = 'dynamic'
                    AND lower(?5) <> 'dynamic'
                    AND lower(?6) <> 'dynamic'
                    AND EXISTS (
                        SELECT 1
                        FROM chat_sessions dynamic_sessions
                        WHERE dynamic_sessions.id = chat_turns.session_id
                          AND dynamic_sessions.workspace_id = chat_turns.workspace_id
                          AND (
                                (
                                    lower(dynamic_sessions.provider_id) = 'dynamic'
                                    AND lower(dynamic_sessions.model_id) = 'dynamic'
                                )
                                OR COALESCE(dynamic_sessions.dynamic_routing_override, 0) = 1
                          )
                    )
                 )
                 OR (
                        lower(provider_id) <> 'dynamic'
                    AND lower(model_id) <> 'dynamic'
                    AND lower(?5) <> 'dynamic'
                    AND lower(?6) <> 'dynamic'
                    AND EXISTS (
                        SELECT 1
                        FROM chat_sessions retry_sessions
                        WHERE retry_sessions.id = chat_turns.session_id
                          AND retry_sessions.workspace_id = chat_turns.workspace_id
                          AND (
                                (
                                    lower(retry_sessions.provider_id) = 'dynamic'
                                    AND lower(retry_sessions.model_id) = 'dynamic'
                                )
                                OR COALESCE(retry_sessions.dynamic_routing_override, 0) = 1
                          )
                    )
                 )
              )
              AND EXISTS (
                    SELECT 1
                    FROM chat_sessions sessions
                    WHERE sessions.id = chat_turns.session_id
                      AND sessions.workspace_id = chat_turns.workspace_id
                      AND sessions.agent_id = chat_turns.agent_id
              )
            ",
            params![
                context.turn_id,
                context.generation_token,
                workspace_id,
                context.session_id,
                context.provider_id,
                context.model_id,
                context.agent_id,
                context.root_turn_id,
                context.turn_kind,
                claimed_at_ms,
                context.parent_turn_id,
            ],
        )?;
        if changed == 1 {
            rebind_claimed_user_message_route(&transaction, &workspace_id, context)?;
            transaction.commit()?;
            return Ok(());
        }
        let already_claimed =
            existing_response_claim_matches(&transaction, &workspace_id, context)?;
        Err(rusqlite::Error::InvalidParameterName(
            if already_claimed {
                CHAT_TURN_RESPONSE_CLAIM_CONFLICT
            } else {
                CHAT_TURN_RESPONSE_CLAIM_MISMATCH
            }
            .to_string(),
        ))
    }
}
