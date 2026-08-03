use super::*;

impl PersistenceEngine {
    pub fn insert_queued_message(
        &self,
        request: QueueMessageRequest,
    ) -> rusqlite::Result<QueuedMessageRecord> {
        let (agent_id, message, attachments_json) = validated_payload(&request)?;
        let turn_id = clean_optional_text(request.turn_id);
        let generation_token = clean_optional_text(request.generation_token);
        let parent_turn_id = clean_optional_text(request.parent_turn_id);
        let root_turn_id = clean_optional_text(request.root_turn_id);
        let turn_kind = clean_optional_text(request.turn_kind);
        let session_id = clean_optional_text(request.session_id);
        let mut provider_id = clean_optional_text(request.provider_id);
        let mut model_id = clean_optional_text(request.model_id);
        let reasoning = clean_optional_text(request.reasoning);
        let context = clean_optional_text(request.context).or_else(|| {
            request
                .context_budget
                .filter(|value| *value > 0)
                .map(|value| value.to_string())
        });
        let steering = clean_optional_text(request.steering);
        let mut auto_route_identity = None;
        let required = |field: &str, value: &Option<String>| -> rusqlite::Result<String> {
            value.clone().ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(format!(
                    "Queued messages require immutable {field}."
                ))
            })
        };
        let mut turn_context = ChatTurnPersistenceContext {
            turn_id: required("turn_id", &turn_id)?,
            generation_token: required("generation_token", &generation_token)?,
            session_id: required("session_id", &session_id)?,
            agent_id: agent_id.clone(),
            provider_id: required("provider_id", &provider_id)?,
            model_id: required("model_id", &model_id)?,
            parent_turn_id: parent_turn_id.clone(),
            root_turn_id: required("root_turn_id", &root_turn_id)?,
            turn_kind: required("turn_kind", &turn_kind)?,
        };
        validate_chat_turn_context_fields(&turn_context)?;
        let now = unix_time_ms();
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id = workspace_id_for_chat_session(
            &connection,
            &turn_context.session_id,
            &self.workspace_id,
        )?;
        let session_agent_id: String = connection.query_row(
            "SELECT agent_id FROM chat_sessions WHERE id = ?1 AND workspace_id = ?2",
            params![turn_context.session_id, workspace_id],
            |row| row.get(0),
        )?;
        if session_agent_id != turn_context.agent_id {
            return Err(rusqlite::Error::InvalidParameterName(
                "queued chat turn agent_id does not own the requested session".to_string(),
            ));
        }
        if let Some(parent_turn_id) = turn_context.parent_turn_id.clone() {
            freeze_parent_turn_identity(&connection, &mut turn_context, &parent_turn_id)?;
            provider_id = Some(turn_context.provider_id.clone());
            model_id = Some(turn_context.model_id.clone());
        } else if turn_context.provider_id.eq_ignore_ascii_case("dynamic")
            || turn_context.model_id.eq_ignore_ascii_case("dynamic")
        {
            auto_route_identity = Some(auto_route::freeze_queued_auto_route_baseline(
                &connection,
                &workspace_id,
                &mut turn_context,
                &mut provider_id,
                &mut model_id,
            )?);
        }
        let auto_route_identity_json = auto_route_identity
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(json_to_sql_error)?;
        connection.execute(
            "INSERT INTO message_queue (
                turn_id, generation_token, parent_turn_id, root_turn_id, turn_kind,
                session_id, agent_id, message, attachments_json, provider_id, model_id,
                reasoning, context_limit, steering, automated_web_grounding_enabled,
                dynamic_routing_override, auto_route_identity_json, status,
                created_at_ms, updated_at_ms
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, 'queued', ?18, ?18
             )",
            params![
                turn_id,
                generation_token,
                parent_turn_id,
                root_turn_id,
                turn_kind,
                session_id,
                agent_id,
                message,
                attachments_json,
                provider_id,
                model_id,
                reasoning,
                context,
                steering,
                request.automated_web_grounding_enabled,
                request.dynamic_routing_override,
                auto_route_identity_json,
                now
            ],
        )?;
        let queued = select_queued_message_by_id(&connection, connection.last_insert_rowid())?;
        emit_frozen_identity_receipt(&queued);
        Ok(queued)
    }

    pub fn select_queued_messages(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<Vec<QueuedMessageRecord>> {
        let connection = self.open_connection()?;
        select_queued_messages_for_session(&connection, session_id, Some(200))
    }

    pub fn claim_queued_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<QueuedMessageRecord>> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let queued = select_queued_messages_for_session(&transaction, session_id, Some(limit))?;
        let now = unix_time_ms();
        for record in &queued {
            transaction.execute(
                "UPDATE message_queue
                 SET status = 'executing', updated_at_ms = ?2, error_message = NULL
                 WHERE id = ?1 AND status = 'queued'",
                params![record.id, now],
            )?;
        }
        transaction.commit()?;
        Ok(queued)
    }

    pub fn mark_queued_message_completed(&self, id: i64) -> rusqlite::Result<()> {
        self.mark_queued_message_terminal(id, "completed", None)
    }

    pub fn mark_queued_message_failed(&self, id: i64, error_message: &str) -> rusqlite::Result<()> {
        self.mark_queued_message_terminal(id, "failed", Some(error_message.trim()))
    }

    fn mark_queued_message_terminal(
        &self,
        id: i64,
        status: &str,
        error_message: Option<&str>,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let now = unix_time_ms();
        connection.execute(
            "UPDATE message_queue
             SET status = ?2, updated_at_ms = ?3, executed_at_ms = ?3, error_message = ?4
             WHERE id = ?1",
            params![id, status, now, error_message],
        )?;
        Ok(())
    }
}

fn validated_payload(request: &QueueMessageRequest) -> rusqlite::Result<(String, String, String)> {
    crate::inference::validate_chat_attachments(&request.attachments)
        .map_err(|code| rusqlite::Error::InvalidParameterName(code.to_string()))?;
    let agent_id = request.agent_id.trim().to_string();
    let message = request.message.trim().to_string();
    if agent_id.is_empty() || message.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Queued messages require agent_id and message.".to_string(),
        ));
    }
    let attachments_json =
        serde_json::to_string(&request.attachments).map_err(json_to_sql_error)?;
    Ok((agent_id, message, attachments_json))
}

fn emit_frozen_identity_receipt(queued: &QueuedMessageRecord) {
    if let (Some(turn_id), Some(session_id), Some(identity)) = (
        queued.turn_id.as_deref(),
        queued.session_id.as_deref(),
        queued.auto_route_identity.as_ref(),
    ) {
        auto_route::emit_queued_auto_route_identity_receipt(turn_id, session_id, identity);
    }
}

fn freeze_parent_turn_identity(
    connection: &Connection,
    turn_context: &mut ChatTurnPersistenceContext,
    parent_turn_id: &str,
) -> rusqlite::Result<()> {
    let parent = connection
        .query_row(
            "SELECT session_id, agent_id, root_turn_id, provider_id, model_id
             FROM chat_turns WHERE turn_id = ?1",
            params![parent_turn_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((parent_session, parent_agent, parent_root, parent_provider, parent_model)) = parent
    else {
        return Err(rusqlite::Error::InvalidParameterName(
            "queued chat turn parent does not exist".to_string(),
        ));
    };
    let provider_matches = turn_context.provider_id.eq_ignore_ascii_case("dynamic")
        || turn_context.provider_id == parent_provider;
    let model_matches = turn_context.model_id.eq_ignore_ascii_case("dynamic")
        || turn_context.model_id == parent_model;
    if parent_session != turn_context.session_id
        || parent_agent != turn_context.agent_id
        || parent_root != turn_context.root_turn_id
        || !provider_matches
        || !model_matches
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "queued chat turn parent crosses a session, agent, root, provider, or model boundary"
                .to_string(),
        ));
    }
    turn_context.provider_id = parent_provider;
    turn_context.model_id = parent_model;
    Ok(())
}

fn queued_message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedMessageRecord> {
    let attachments = queued_attachments_from_row(row)?;
    let auto_route_identity = queued_auto_route_identity_from_row(row)?;
    Ok(QueuedMessageRecord {
        id: row.get(0)?,
        turn_id: row.get(1)?,
        generation_token: row.get(2)?,
        parent_turn_id: row.get(3)?,
        root_turn_id: row.get(4)?,
        turn_kind: row.get(5)?,
        session_id: row.get(6)?,
        agent_id: row.get(7)?,
        message: row.get(8)?,
        attachments,
        provider_id: row.get(10)?,
        model_id: row.get(11)?,
        reasoning: row.get(12)?,
        context: row.get(13)?,
        steering: row.get(14)?,
        automated_web_grounding_enabled: row.get(15)?,
        dynamic_routing_override: row.get(16)?,
        auto_route_identity,
        status: row.get(18)?,
        created_at_ms: row.get(19)?,
        updated_at_ms: row.get(20)?,
        executed_at_ms: row.get(21)?,
        error_message: row.get(22)?,
    })
}

fn queued_attachments_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Vec<crate::inference::ChatAttachment>> {
    let value: String = row.get(9)?;
    serde_json::from_str(&value).map_err(json_from_sql_error)
}

fn queued_auto_route_identity_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Option<QueuedAutoRouteIdentityRecord>> {
    row.get::<_, Option<String>>(17)?
        .map(|value| serde_json::from_str(&value).map_err(json_from_sql_error))
        .transpose()
}

fn select_queued_message_by_id(
    connection: &Connection,
    id: i64,
) -> rusqlite::Result<QueuedMessageRecord> {
    connection.query_row(
        &format!("{} WHERE id = ?1", queued_message_select()),
        params![id],
        queued_message_from_row,
    )
}

fn select_queued_messages_for_session(
    connection: &Connection,
    session_id: &str,
    limit: Option<usize>,
) -> rusqlite::Result<Vec<QueuedMessageRecord>> {
    let limit = limit.unwrap_or(200).clamp(1, 1_000) as i64;
    let cleaned_session_id = session_id.trim();
    if cleaned_session_id.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "queued message reads require a non-empty session_id".to_string(),
        ));
    }
    let sql = format!(
        "{} WHERE status = 'queued' AND session_id = ?1
         ORDER BY created_at_ms ASC, id ASC LIMIT ?2",
        queued_message_select()
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![cleaned_session_id, limit], queued_message_from_row)?;
    rows.collect()
}

fn queued_message_select() -> &'static str {
    "SELECT id, turn_id, generation_token, parent_turn_id, root_turn_id, turn_kind,
            session_id, agent_id, message, attachments_json, provider_id, model_id,
            reasoning, context_limit, steering, automated_web_grounding_enabled,
            dynamic_routing_override, auto_route_identity_json, status,
            created_at_ms, updated_at_ms, executed_at_ms, error_message
     FROM message_queue"
}

fn clean_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
