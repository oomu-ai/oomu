use super::*;
use std::path::Path;

#[derive(Clone)]
struct AutoRouteActivationState {
    session_provider_id: String,
    session_model_id: String,
    dynamic_routing_override: Option<bool>,
    provider_config_id: Option<String>,
    provider_type: Option<String>,
    model_id: Option<String>,
    reasoning_depth: Option<String>,
    context_budget: Option<i32>,
    provenance: Option<String>,
    route_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutoRoutePersistedStateEvidence {
    pub state_digest: String,
    pub route_generation: RouteGeneration,
    pub dynamic_routing_enabled: bool,
}

struct AutoRouteCommittedMutation {
    provider_config_id: ProviderConfigurationId,
    provider_type: ProviderTypeId,
    model_id: CanonicalModelId,
    provenance: AutoRouteProvenance,
    current_generation: RouteGeneration,
    dynamic_routing_enabled: bool,
    changed: bool,
}

struct FrozenAutoRouteTurnState {
    status: String,
    provider_id: String,
    model_id: String,
    claimed_at_ms: Option<i64>,
    message_id: i64,
    metadata_json: Option<String>,
}

impl PersistenceEngine {
    pub(crate) fn resumable_auto_route_turn_policy(
        &self,
        turn_id: &str,
        generation_token: &str,
        session_id: &str,
        agent_id: &str,
    ) -> rusqlite::Result<Option<AutoRouteTurnPolicyRecord>> {
        let (turn_id, generation_token, session_id, agent_id) =
            validate_frozen_policy_identity(turn_id, generation_token, session_id, agent_id)?;
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?;
        let Some(state) = find_frozen_auto_route_turn_state(
            &connection,
            turn_id,
            generation_token,
            &workspace_id,
            session_id,
            agent_id,
        )?
        else {
            return Ok(None);
        };
        let metadata = state
            .metadata_json
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(json_to_sql_error)?
            .unwrap_or_else(|| json!({}));
        let Some(policy) = metadata.get("autoRoutePolicy") else {
            return Ok(None);
        };
        if state.status != "running" || state.claimed_at_ms.is_some() {
            return Err(rusqlite::Error::InvalidParameterName(
                "Frozen Auto-route continuation requires an unclaimed running turn.".to_string(),
            ));
        }
        serde_json::from_value(policy.clone())
            .map(Some)
            .map_err(json_to_sql_error)
    }

    pub(crate) fn restore_verified_dynamic_session_binding(
        &self,
        session_id: &str,
        expected_session_provider_id: &str,
        expected_session_model_id: &str,
        local_provider_config_id: &str,
        local_provider_type: &str,
        local_model_id: &str,
        route_generation: i64,
    ) -> rusqlite::Result<bool> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let verified_config_exists: bool = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM active_session_configs
                 WHERE session_id=?1 AND local_provider_config_id=?2
                   AND local_provider_type=?3 AND model_id=?4
                   AND local_route_generation=?5 AND local_route_generation>0
             )",
            params![
                session_id,
                local_provider_config_id,
                local_provider_type,
                local_model_id,
                route_generation,
            ],
            |row| row.get(0),
        )?;
        if !verified_config_exists {
            return Err(rusqlite::Error::InvalidParameterName(
                "auto_route_session_binding_repair_precondition_failed".to_string(),
            ));
        }
        let now = unix_time_ms();
        let changed = transaction.execute(
            "UPDATE chat_sessions
             SET provider_id='dynamic',model_id='dynamic',dynamic_routing_override=1,
                 updated_at_ms=?1
             WHERE id=?2 AND workspace_id=?3 AND provider_id=?4 AND model_id=?5",
            params![
                now,
                session_id,
                &self.workspace_id,
                expected_session_provider_id,
                expected_session_model_id,
            ],
        )? == 1;
        if !changed {
            let binding: (String, String) = transaction.query_row(
                "SELECT provider_id,model_id FROM chat_sessions
                 WHERE id=?1 AND workspace_id=?2",
                params![session_id, &self.workspace_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if !binding.0.eq_ignore_ascii_case("dynamic")
                || !binding.1.eq_ignore_ascii_case("dynamic")
            {
                return Err(rusqlite::Error::InvalidParameterName(
                    "auto_route_session_binding_changed_during_repair".to_string(),
                ));
            }
        }
        transaction.commit()?;
        crate::diagnostic_output::write_diagnostic_line(format_args!(
            "OOMU_NATIVE_RECEIPT {}",
            serde_json::json!({
                "kind": "auto_route_binding_repair",
                "receiptId": format!("auto-route-binding-repair-{session_id}-{route_generation}"),
                "sessionId": session_id,
                "providerConfigId": local_provider_config_id,
                "providerType": local_provider_type,
                "modelId": local_model_id,
                "routeGeneration": route_generation,
                "committed": true,
                "changed": changed,
                "rolledBack": false,
            })
        ));
        Ok(changed)
    }

    pub fn auto_route_audit_storage_ready(&self) -> rusqlite::Result<bool> {
        let connection = self.open_ops_connection()?;
        connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'local_inference_audit'
             )",
            [],
            |row| row.get(0),
        )
    }

    pub(crate) fn ensure_chat_session_with_auto_route_baseline(
        &self,
        request: CreateChatSessionRequest,
        baseline: VerifiedAutoRouteBaseline,
        model_root: &Path,
    ) -> rusqlite::Result<ChatSessionRecord> {
        if request.provider_id.trim() != "dynamic" || request.model_id.trim() != "dynamic" {
            return Err(rusqlite::Error::InvalidParameterName(
                "Auto-route baseline requires a dynamic/dynamic chat session binding.".to_string(),
            ));
        }
        let provider_config_id = baseline.provider_config_id.as_str();
        let provider_type = baseline.provider_type.as_str();
        let model_id = baseline.model_id.as_str();
        let reasoning_depth = baseline.reasoning_depth.trim();
        if provider_config_id.is_empty()
            || provider_type.is_empty()
            || model_id.is_empty()
            || reasoning_depth.is_empty()
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "Auto-route baseline requires concrete provider, model, and reasoning values."
                    .to_string(),
            ));
        }
        if provider_config_id == "dynamic"
            || provider_type == "dynamic"
            || model_id == "dynamic"
            || baseline.context_budget <= 0
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "Auto-route baseline must be a concrete local route with a positive context budget."
                    .to_string(),
            ));
        }
        let canonical_model_id = super::auto_route_validation::canonical_ready_local_baseline(
            model_root,
            provider_type,
            model_id,
        )?;

        let now = unix_time_ms();
        let workspace_id =
            workspace_id_from_request(request.workspace_id.as_deref(), &self.workspace_id)?;
        let id = format!("chat-session-{now}");
        let title = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("New Session")
            .to_string();
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "
            INSERT INTO chat_sessions (
                id, workspace_id, agent_id, title, title_source, provider_id, model_id,
                dynamic_routing_override, created_at_ms, updated_at_ms
            )
            VALUES (?1, ?2, ?3, ?4, 'auto', 'dynamic', 'dynamic', 1, ?5, ?5)
            ",
            params![&id, &workspace_id, &request.agent_id, &title, now],
        )?;
        transaction.execute(
            "
            INSERT INTO active_session_configs (
                session_id, reasoning_depth, context_budget, model_id, provider_id, updated_at,
                local_model_source, local_model_reconciled_at_ms, local_provider_config_id,
                local_provider_type, local_route_generation
            )
            VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP, ?6, ?7, ?8, ?9, 1)
            ",
            params![
                &id,
                reasoning_depth,
                baseline.context_budget,
                canonical_model_id,
                provider_config_id,
                baseline.provenance.as_str(),
                now,
                provider_config_id,
                provider_type,
            ],
        )?;
        transaction.commit()?;
        Ok(ChatSessionRecord {
            id,
            workspace_id,
            project_id: None,
            agent_id: request.agent_id,
            title,
            title_source: "auto".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            web_grounding_override: None,
            dynamic_routing_override: Some(true),
            unread_completion: false,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub fn select_chat_session_route_policy(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<Option<ChatSessionRoutePolicyRecord>> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Ok(None);
        }
        let connection = self.open_connection()?;
        connection
            .query_row(
                "
                SELECT sessions.id, sessions.agent_id, sessions.provider_id,
                       sessions.model_id, sessions.dynamic_routing_override,
                       config.provider_id, config.model_id, config.reasoning_depth,
                       config.context_budget, config.updated_at,
                       config.local_model_source, config.local_model_reconciled_at_ms,
                       config.local_provider_config_id, config.local_provider_type,
                       COALESCE(config.local_route_generation, 0)
                FROM chat_sessions sessions
                LEFT JOIN active_session_configs config ON config.session_id = sessions.id
                WHERE sessions.id = ?1 AND sessions.workspace_id = ?2
                ",
                params![session_id, &self.workspace_id],
                |row| {
                    Ok(ChatSessionRoutePolicyRecord {
                        session_id: row.get(0)?,
                        agent_id: row.get(1)?,
                        session_provider_id: row.get(2)?,
                        session_model_id: row.get(3)?,
                        dynamic_routing_override: row.get(4)?,
                        local_provider_id: row.get(12)?,
                        local_provider_type: row.get(13)?,
                        local_model_id: row.get(6)?,
                        reasoning_depth: row.get(7)?,
                        context_budget: row.get(8)?,
                        baseline_updated_at: row.get(9)?,
                        local_source: row.get(10)?,
                        local_reconciled_at_ms: row.get(11)?,
                        route_generation: row.get(14)?,
                    })
                },
            )
            .optional()
    }

    #[cfg(test)]
    pub(crate) fn update_chat_session_dynamic_routing_override(
        &self,
        session_id: &str,
        dynamic_routing_override: Option<bool>,
        verified_baseline: Option<VerifiedAutoRouteBaseline>,
        model_root: Option<&Path>,
    ) -> rusqlite::Result<AutoRouteActivationResponse> {
        let _guard = self.lock_writes();
        self.update_chat_session_dynamic_routing_override_locked(
            session_id,
            dynamic_routing_override,
            verified_baseline,
            model_root,
        )
    }

    pub(crate) fn update_chat_session_dynamic_routing_override_locked(
        &self,
        session_id: &str,
        dynamic_routing_override: Option<bool>,
        verified_baseline: Option<VerifiedAutoRouteBaseline>,
        model_root: Option<&Path>,
    ) -> rusqlite::Result<AutoRouteActivationResponse> {
        let mut connection = self.open_connection()?;
        let workspace_id = self.workspace_id.clone();
        let now = unix_time_ms();
        let transaction = connection.transaction()?;
        let state = load_auto_route_activation_state(&transaction, session_id, &workspace_id)?;
        let previous = auto_route_state_evidence(session_id, &workspace_id, &state);
        let previous_generation = previous.route_generation;
        let mutation = if dynamic_routing_override == Some(true) {
            commit_auto_route_enabled(
                &transaction,
                session_id,
                &workspace_id,
                now,
                &state,
                previous_generation,
                verified_baseline,
                model_root,
            )?
        } else {
            commit_auto_route_disabled(
                &transaction,
                session_id,
                &workspace_id,
                now,
                &state,
                previous_generation,
                dynamic_routing_override,
                verified_baseline,
            )?
        };
        transaction.commit()?;
        let session = load_committed_chat_session(&connection, session_id, &workspace_id)?;
        let current_state =
            load_auto_route_activation_state(&connection, session_id, &workspace_id)?;
        let current = auto_route_state_evidence(session_id, &workspace_id, &current_state);
        verify_committed_activation_evidence(&previous, &current, &mutation)?;
        let receipt = activation_receipt(session_id, previous, current, mutation);
        emit_auto_route_receipt(&receipt);
        Ok(AutoRouteActivationResponse { session, receipt })
    }

    /// Reads the exact saved manual route that Auto-route would restore.
    /// The caller must hold both the provider and persistence write locks so
    /// this request can be verified against the provider store before commit.
    pub(crate) fn saved_auto_route_baseline_request_locked(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<AutoRouteSessionBaselineRequest> {
        let connection = self.open_connection()?;
        let state = load_auto_route_activation_state(&connection, session_id, &self.workspace_id)?;
        let reasoning_depth = state
            .reasoning_depth
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("auto_route_baseline_incomplete".to_string())
            })?
            .to_string();
        let context_budget = state
            .context_budget
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("auto_route_baseline_incomplete".to_string())
            })?;
        Ok(AutoRouteSessionBaselineRequest {
            provider_config_id: required_provider_configuration(&state)?,
            provider_type: required_provider_type(&state)?,
            model_id: required_model_id(&state)?,
            reasoning_depth,
            context_budget,
        })
    }

    pub fn freeze_auto_route_turn_policy(
        &self,
        turn_id: &str,
        generation_token: &str,
        session_id: &str,
        agent_id: &str,
        policy: AutoRouteTurnPolicyRecord,
    ) -> rusqlite::Result<AutoRouteTurnPolicyRecord> {
        let (turn_id, generation_token, session_id, agent_id) = validate_frozen_policy_request(
            turn_id,
            generation_token,
            session_id,
            agent_id,
            &policy,
        )?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, session_id, &self.workspace_id)?;
        let state = load_frozen_auto_route_turn_state(
            &transaction,
            turn_id,
            generation_token,
            &workspace_id,
            session_id,
            agent_id,
        )?;
        let mut metadata = state
            .metadata_json
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(json_to_sql_error)?
            .unwrap_or_else(|| json!({}));
        if let Some(existing) = metadata.get("autoRoutePolicy") {
            let existing = serde_json::from_value(existing.clone()).map_err(json_to_sql_error)?;
            transaction.commit()?;
            return Ok(existing);
        }
        verify_current_auto_route_baseline(&transaction, session_id, &policy)?;
        if state.status != "running"
            || state.claimed_at_ms.is_some()
            || !state.provider_id.eq_ignore_ascii_case("dynamic")
            || !state.model_id.eq_ignore_ascii_case("dynamic")
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "Auto-route turn policy must freeze before provider selection or dispatch."
                    .to_string(),
            ));
        }
        metadata
            .as_object_mut()
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(
                    "Accepted turn metadata must be a JSON object.".to_string(),
                )
            })?
            .insert(
                "autoRoutePolicy".to_string(),
                serde_json::to_value(&policy).map_err(json_to_sql_error)?,
            );
        if transaction.execute(
            "UPDATE chat_messages SET metadata_json = ?2 WHERE id = ?1",
            params![state.message_id, metadata.to_string()],
        )? != 1
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "Auto-route turn policy did not bind to exactly one accepted message.".to_string(),
            ));
        }
        transaction.commit()?;
        emit_frozen_auto_route_policy_receipt(turn_id, session_id, &policy);
        Ok(policy)
    }

    pub fn insert_dynamic_routing_audit(
        &self,
        prompt: &str,
        output: &str,
        metadata: &Value,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_ops_connection()?;
        ensure_local_inference_audit_schema(&connection)?;
        let metadata_json = serde_json::to_string(metadata).map_err(json_to_sql_error)?;
        let prompt_hash = sha256_hex(prompt.as_bytes());
        let output_hash = sha256_hex(output.as_bytes());
        let trace_hash = sha256_hex(metadata_json.as_bytes());
        connection.execute(
            "INSERT INTO local_inference_audit (
                event_id, event_kind, prompt_hash, output_hash, trace_hash,
                metadata_json, created_at_ms
             ) VALUES (?1, 'dynamic_routing', ?2, ?3, ?4, ?5, ?6)",
            params![
                format!(
                    "dynamic-routing-{}-{}",
                    unix_time_ms(),
                    output_hash.chars().take(12).collect::<String>()
                ),
                prompt_hash,
                output_hash,
                trace_hash,
                metadata_json,
                unix_time_ms(),
            ],
        )?;
        crate::diagnostic_output::write_functional_acceptance_receipt(metadata);
        Ok(())
    }
}

fn validate_frozen_policy_request<'a>(
    turn_id: &'a str,
    generation_token: &'a str,
    session_id: &'a str,
    agent_id: &'a str,
    policy: &AutoRouteTurnPolicyRecord,
) -> rusqlite::Result<(&'a str, &'a str, &'a str, &'a str)> {
    let values = validate_frozen_policy_identity(turn_id, generation_token, session_id, agent_id)?;
    let identity_incomplete = policy.local_provider_id.trim().is_empty()
        || policy.local_provider_type.trim().is_empty()
        || policy.local_model_id.trim().is_empty()
        || policy.local_reasoning.trim().is_empty()
        || policy.local_context_budget <= 0
        || policy.route_generation <= 0
        || policy.classifier_version.trim().is_empty()
        || policy.policy_version.trim().is_empty();
    if identity_incomplete {
        return Err(rusqlite::Error::InvalidParameterName(
            "Auto-route turn policy requires a complete immutable identity and local baseline."
                .to_string(),
        ));
    }
    Ok(values)
}

fn validate_frozen_policy_identity<'a>(
    turn_id: &'a str,
    generation_token: &'a str,
    session_id: &'a str,
    agent_id: &'a str,
) -> rusqlite::Result<(&'a str, &'a str, &'a str, &'a str)> {
    let values = (
        turn_id.trim(),
        generation_token.trim(),
        session_id.trim(),
        agent_id.trim(),
    );
    if [values.0, values.1, values.2, values.3]
        .iter()
        .any(|value| value.is_empty())
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "Auto-route turn policy requires a complete immutable turn identity.".to_string(),
        ));
    }
    Ok(values)
}

fn verify_current_auto_route_baseline(
    transaction: &rusqlite::Transaction<'_>,
    session_id: &str,
    policy: &AutoRouteTurnPolicyRecord,
) -> rusqlite::Result<()> {
    let current = transaction.query_row(
        "SELECT local_provider_config_id, local_provider_type, model_id,
                local_model_source, local_route_generation
         FROM active_session_configs WHERE session_id = ?1",
        params![session_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        },
    )?;
    let stale = current.0.as_deref() != Some(policy.local_provider_id.as_str())
        || current.1.as_deref() != Some(policy.local_provider_type.as_str())
        || current.2.as_deref() != Some(policy.local_model_id.as_str())
        || current.3 != policy.local_source
        || current.4 != policy.route_generation;
    if stale {
        return Err(rusqlite::Error::InvalidParameterName(
            "auto_route_route_generation_stale".to_string(),
        ));
    }
    Ok(())
}

fn load_frozen_auto_route_turn_state(
    transaction: &rusqlite::Transaction<'_>,
    turn_id: &str,
    generation_token: &str,
    workspace_id: &str,
    session_id: &str,
    agent_id: &str,
) -> rusqlite::Result<FrozenAutoRouteTurnState> {
    find_frozen_auto_route_turn_state(
        transaction,
        turn_id,
        generation_token,
        workspace_id,
        session_id,
        agent_id,
    )?
    .ok_or_else(|| {
        rusqlite::Error::InvalidParameterName(
            "Auto-route turn policy can only freeze an accepted root turn.".to_string(),
        )
    })
}

fn find_frozen_auto_route_turn_state(
    connection: &Connection,
    turn_id: &str,
    generation_token: &str,
    workspace_id: &str,
    session_id: &str,
    agent_id: &str,
) -> rusqlite::Result<Option<FrozenAutoRouteTurnState>> {
    connection
        .query_row(
            "SELECT turns.status, turns.provider_id, turns.model_id,
                    turns.response_claimed_at_ms, messages.id, messages.metadata_json
             FROM chat_turns turns
             JOIN chat_sessions sessions ON sessions.id = turns.session_id
                AND sessions.workspace_id = turns.workspace_id
                AND sessions.agent_id = turns.agent_id
             JOIN chat_messages messages ON messages.workspace_id = turns.workspace_id
                AND messages.session_id = turns.session_id AND messages.role = 'user'
                AND json_extract(messages.metadata_json, '$.turnId') = turns.turn_id
             WHERE turns.turn_id = ?1 AND turns.generation_token = ?2
               AND turns.workspace_id = ?3 AND turns.session_id = ?4
               AND turns.agent_id = ?5 AND turns.parent_turn_id IS NULL
               AND turns.root_turn_id = turns.turn_id AND turns.turn_kind = 'root'
             ORDER BY messages.id ASC LIMIT 1",
            params![
                turn_id,
                generation_token,
                workspace_id,
                session_id,
                agent_id
            ],
            |row| {
                Ok(FrozenAutoRouteTurnState {
                    status: row.get(0)?,
                    provider_id: row.get(1)?,
                    model_id: row.get(2)?,
                    claimed_at_ms: row.get(3)?,
                    message_id: row.get(4)?,
                    metadata_json: row.get(5)?,
                })
            },
        )
        .optional()
}

fn emit_frozen_auto_route_policy_receipt(
    turn_id: &str,
    session_id: &str,
    policy: &AutoRouteTurnPolicyRecord,
) {
    crate::diagnostic_output::write_diagnostic_line(format_args!(
        "OOMU_NATIVE_RECEIPT {}",
        serde_json::json!({
            "kind": "auto_route_turn_policy_frozen",
            "receiptId": format!("auto-route-turn-{turn_id}-{}", policy.route_generation),
            "sessionId": session_id,
            "turnId": turn_id,
            "providerConfigId": policy.local_provider_id,
            "providerType": policy.local_provider_type,
            "modelId": policy.local_model_id,
            "provenance": policy.local_source,
            "routeGeneration": policy.route_generation,
            "committed": true,
            "rolledBack": false,
            "retryable": false,
        })
    ));
}

fn load_auto_route_activation_state(
    connection: &Connection,
    session_id: &str,
    workspace_id: &str,
) -> rusqlite::Result<AutoRouteActivationState> {
    connection
        .query_row(
            "SELECT sessions.provider_id, sessions.model_id,
                    sessions.dynamic_routing_override,
                    config.local_provider_config_id, config.local_provider_type,
                    config.model_id, config.reasoning_depth, config.context_budget,
                    config.local_model_source, config.local_route_generation
             FROM chat_sessions sessions
             LEFT JOIN active_session_configs config ON config.session_id = sessions.id
             WHERE sessions.id = ?1 AND sessions.workspace_id = ?2",
            params![session_id, workspace_id],
            |row| {
                Ok(AutoRouteActivationState {
                    session_provider_id: row.get(0)?,
                    session_model_id: row.get(1)?,
                    dynamic_routing_override: row.get(2)?,
                    provider_config_id: row.get(3)?,
                    provider_type: row.get(4)?,
                    model_id: row.get(5)?,
                    reasoning_depth: row.get(6)?,
                    context_budget: row.get(7)?,
                    provenance: row.get(8)?,
                    route_generation: row.get::<_, Option<i64>>(9)?.unwrap_or(0),
                })
            },
        )
        .optional()?
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub(crate) fn read_persisted_auto_route_state(
    engine: &PersistenceEngine,
    session_id: &str,
) -> rusqlite::Result<AutoRoutePersistedStateEvidence> {
    let connection = engine.open_connection()?;
    let state = load_auto_route_activation_state(&connection, session_id, &engine.workspace_id)?;
    Ok(auto_route_state_evidence(
        session_id,
        &engine.workspace_id,
        &state,
    ))
}

pub(crate) fn missing_auto_route_state_digest(session_id: &str) -> String {
    sha256_hex(format!("oomu-auto-route-state-v1\0missing\0{}", session_id.trim()).as_bytes())
}

fn auto_route_state_evidence(
    session_id: &str,
    workspace_id: &str,
    state: &AutoRouteActivationState,
) -> AutoRoutePersistedStateEvidence {
    let document = canonicalize_json(&serde_json::json!({
        "schema": "oomu.auto-route-state.v1",
        "sessionId": session_id,
        "workspaceId": workspace_id,
        "sessionProviderId": state.session_provider_id,
        "sessionModelId": state.session_model_id,
        "dynamicRoutingOverride": state.dynamic_routing_override,
        "providerConfigId": state.provider_config_id,
        "providerType": state.provider_type,
        "modelId": state.model_id,
        "reasoningDepth": state.reasoning_depth,
        "contextBudget": state.context_budget,
        "provenance": state.provenance,
        "routeGeneration": state.route_generation,
    }));
    AutoRoutePersistedStateEvidence {
        state_digest: sha256_hex(document.to_string().as_bytes()),
        route_generation: RouteGeneration::from_persisted(state.route_generation),
        dynamic_routing_enabled: state.session_provider_id.eq_ignore_ascii_case("dynamic")
            && state.session_model_id.eq_ignore_ascii_case("dynamic")
            && state.dynamic_routing_override == Some(true),
    }
}

fn verify_committed_activation_evidence(
    previous: &AutoRoutePersistedStateEvidence,
    current: &AutoRoutePersistedStateEvidence,
    mutation: &AutoRouteCommittedMutation,
) -> rusqlite::Result<()> {
    let generation_valid = current.route_generation == mutation.current_generation
        && if mutation.changed {
            current.route_generation.get() > previous.route_generation.get()
                && current.state_digest != previous.state_digest
        } else {
            current.route_generation == previous.route_generation
                && current.state_digest == previous.state_digest
        };
    if !generation_valid || current.dynamic_routing_enabled != mutation.dynamic_routing_enabled {
        return Err(rusqlite::Error::InvalidParameterName(
            "auto_route_activation_postcondition_unverified".to_string(),
        ));
    }
    Ok(())
}

fn commit_auto_route_enabled(
    transaction: &rusqlite::Transaction<'_>,
    session_id: &str,
    workspace_id: &str,
    now: i64,
    state: &AutoRouteActivationState,
    previous_generation: RouteGeneration,
    baseline: Option<VerifiedAutoRouteBaseline>,
    model_root: Option<&Path>,
) -> rusqlite::Result<AutoRouteCommittedMutation> {
    let baseline = baseline.ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("auto_route_baseline_incomplete".to_string())
    })?;
    let root = model_root.ok_or_else(|| {
        rusqlite::Error::InvalidParameterName(
            "auto_route_local_model_store_unavailable".to_string(),
        )
    })?;
    let canonical_model = super::auto_route_validation::canonical_ready_local_baseline(
        root,
        baseline.provider_type.as_str(),
        baseline.model_id.as_str(),
    )?;
    let canonical_model_id = CanonicalModelId::try_from(canonical_model)
        .map_err(rusqlite::Error::InvalidParameterName)?;
    let already_committed = enabled_route_matches(state, &baseline, &canonical_model_id)
        && previous_generation.get() > 0;
    let current_generation = if already_committed {
        previous_generation
    } else {
        persist_enabled_route(
            transaction,
            session_id,
            workspace_id,
            now,
            &baseline,
            &canonical_model_id,
            previous_generation.next(),
        )?
    };
    Ok(AutoRouteCommittedMutation {
        provider_config_id: baseline.provider_config_id,
        provider_type: baseline.provider_type,
        model_id: canonical_model_id,
        provenance: baseline.provenance,
        current_generation,
        dynamic_routing_enabled: true,
        changed: !already_committed,
    })
}

fn enabled_route_matches(
    state: &AutoRouteActivationState,
    baseline: &VerifiedAutoRouteBaseline,
    canonical_model_id: &CanonicalModelId,
) -> bool {
    state.session_provider_id.eq_ignore_ascii_case("dynamic")
        && state.session_model_id.eq_ignore_ascii_case("dynamic")
        && state.dynamic_routing_override == Some(true)
        && state.provider_config_id.as_deref() == Some(baseline.provider_config_id.as_str())
        && state.provider_type.as_deref() == Some(baseline.provider_type.as_str())
        && state.model_id.as_deref() == Some(canonical_model_id.as_str())
        && state.reasoning_depth.as_deref() == Some(baseline.reasoning_depth.as_str())
        && state.context_budget == Some(baseline.context_budget)
        && state.provenance.as_deref() == Some(baseline.provenance.as_str())
}

fn persist_enabled_route(
    transaction: &rusqlite::Transaction<'_>,
    session_id: &str,
    workspace_id: &str,
    now: i64,
    baseline: &VerifiedAutoRouteBaseline,
    canonical_model_id: &CanonicalModelId,
    generation: RouteGeneration,
) -> rusqlite::Result<RouteGeneration> {
    transaction.execute(
        "INSERT INTO active_session_configs (
             session_id, reasoning_depth, context_budget, model_id, provider_id,
             updated_at, local_model_source, local_model_reconciled_at_ms,
             local_provider_config_id, local_provider_type, local_route_generation
         ) VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(session_id) DO UPDATE SET
             reasoning_depth=excluded.reasoning_depth, context_budget=excluded.context_budget,
             model_id=excluded.model_id, provider_id=excluded.provider_id,
             local_model_source=excluded.local_model_source,
             local_model_reconciled_at_ms=excluded.local_model_reconciled_at_ms,
             local_provider_config_id=excluded.local_provider_config_id,
             local_provider_type=excluded.local_provider_type,
             local_route_generation=excluded.local_route_generation,
             updated_at=CURRENT_TIMESTAMP",
        params![
            session_id,
            baseline.reasoning_depth,
            baseline.context_budget,
            canonical_model_id.as_str(),
            baseline.provider_config_id.as_str(),
            baseline.provenance.as_str(),
            now,
            baseline.provider_config_id.as_str(),
            baseline.provider_type.as_str(),
            generation.get(),
        ],
    )?;
    if transaction.execute(
        "UPDATE chat_sessions SET provider_id='dynamic',model_id='dynamic',
             dynamic_routing_override=1,updated_at_ms=?1
         WHERE id=?2 AND workspace_id=?3",
        params![now, session_id, workspace_id],
    )? != 1
    {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(generation)
}

fn commit_auto_route_disabled(
    transaction: &rusqlite::Transaction<'_>,
    session_id: &str,
    workspace_id: &str,
    now: i64,
    state: &AutoRouteActivationState,
    previous_generation: RouteGeneration,
    dynamic_routing_override: Option<bool>,
    baseline: Option<VerifiedAutoRouteBaseline>,
) -> rusqlite::Result<AutoRouteCommittedMutation> {
    let baseline = baseline.ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("auto_route_baseline_incomplete".to_string())
    })?;
    let provider_config_id = required_provider_configuration(state)?;
    let provider_type = required_provider_type(state)?;
    let model_id = required_model_id(state)?;
    let saved_reasoning = state.reasoning_depth.as_deref().map(str::trim);
    let verified_identity_changed = provider_config_id != baseline.provider_config_id
        || provider_type != baseline.provider_type
        || model_id != baseline.model_id
        || saved_reasoning != Some(baseline.reasoning_depth.as_str())
        || state.context_budget != Some(baseline.context_budget);
    if verified_identity_changed {
        return Err(rusqlite::Error::InvalidParameterName(
            "auto_route_disable_baseline_changed".to_string(),
        ));
    }
    let already_committed = state.session_provider_id == provider_config_id.as_str()
        && state.session_model_id == model_id.as_str()
        && state.dynamic_routing_override == dynamic_routing_override
        && previous_generation.get() > 0;
    let current_generation = if already_committed {
        previous_generation
    } else {
        persist_disabled_route(
            transaction,
            session_id,
            workspace_id,
            now,
            dynamic_routing_override,
            &provider_config_id,
            &model_id,
            previous_generation.next(),
        )?
    };
    Ok(AutoRouteCommittedMutation {
        provider_config_id,
        provider_type,
        model_id,
        provenance: AutoRouteProvenance::ExplicitSession,
        current_generation,
        dynamic_routing_enabled: false,
        changed: !already_committed,
    })
}

fn required_provider_configuration(
    state: &AutoRouteActivationState,
) -> rusqlite::Result<ProviderConfigurationId> {
    ProviderConfigurationId::try_from(state.provider_config_id.clone().ok_or_else(|| {
        rusqlite::Error::InvalidParameterName(
            "auto_route_provider_configuration_missing".to_string(),
        )
    })?)
    .map_err(rusqlite::Error::InvalidParameterName)
}

fn required_provider_type(state: &AutoRouteActivationState) -> rusqlite::Result<ProviderTypeId> {
    ProviderTypeId::try_from(state.provider_type.clone().ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("auto_route_provider_identity_mismatch".to_string())
    })?)
    .map_err(rusqlite::Error::InvalidParameterName)
}

fn required_model_id(state: &AutoRouteActivationState) -> rusqlite::Result<CanonicalModelId> {
    CanonicalModelId::try_from(state.model_id.clone().ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("auto_route_model_identity_invalid".to_string())
    })?)
    .map_err(rusqlite::Error::InvalidParameterName)
}

fn persist_disabled_route(
    transaction: &rusqlite::Transaction<'_>,
    session_id: &str,
    workspace_id: &str,
    now: i64,
    dynamic_routing_override: Option<bool>,
    provider_config_id: &ProviderConfigurationId,
    model_id: &CanonicalModelId,
    generation: RouteGeneration,
) -> rusqlite::Result<RouteGeneration> {
    if transaction.execute(
        "UPDATE active_session_configs SET local_route_generation=?2,
             updated_at=CURRENT_TIMESTAMP WHERE session_id=?1",
        params![session_id, generation.get()],
    )? != 1
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "auto_route_baseline_incomplete".to_string(),
        ));
    }
    if transaction.execute(
        "UPDATE chat_sessions SET provider_id=?1,model_id=?2,
             dynamic_routing_override=?3,updated_at_ms=?4
         WHERE id=?5 AND workspace_id=?6",
        params![
            provider_config_id.as_str(),
            model_id.as_str(),
            dynamic_routing_override,
            now,
            session_id,
            workspace_id,
        ],
    )? != 1
    {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(generation)
}

fn load_committed_chat_session(
    connection: &Connection,
    session_id: &str,
    workspace_id: &str,
) -> rusqlite::Result<ChatSessionRecord> {
    connection.query_row(
        "SELECT id,workspace_id,project_id,agent_id,title,title_source,
                provider_id,model_id,web_grounding_override,dynamic_routing_override,
                unread_completion,created_at_ms,updated_at_ms
         FROM chat_sessions WHERE id=?1 AND workspace_id=?2",
        params![session_id, workspace_id],
        chat_session_from_row,
    )
}

fn activation_receipt(
    session_id: &str,
    previous: AutoRoutePersistedStateEvidence,
    current: AutoRoutePersistedStateEvidence,
    mutation: AutoRouteCommittedMutation,
) -> AutoRouteActivationReceipt {
    AutoRouteActivationReceipt {
        kind: "auto_route_activation",
        receipt_id: format!(
            "auto-route-{}-{}-{}",
            session_id,
            current.route_generation.get(),
            if current.dynamic_routing_enabled {
                "on"
            } else {
                "off"
            }
        ),
        session_id: session_id.to_string(),
        provider_config_id: Some(mutation.provider_config_id),
        provider_type: Some(mutation.provider_type),
        model_id: Some(mutation.model_id),
        provenance: Some(mutation.provenance),
        previous_route_generation: previous.route_generation,
        current_route_generation: current.route_generation,
        previous_state_digest: previous.state_digest,
        current_state_digest: current.state_digest,
        dynamic_routing_enabled: current.dynamic_routing_enabled,
        changed: mutation.changed,
        committed: true,
        rolled_back: false,
        retryable: false,
        error_code: None,
    }
}

pub(crate) fn emit_auto_route_receipt(receipt: &AutoRouteActivationReceipt) {
    if let Ok(value) = serde_json::to_value(receipt) {
        crate::diagnostic_output::write_diagnostic_line(format_args!(
            "OOMU_NATIVE_RECEIPT {value}"
        ));
    }
}

pub(super) fn freeze_queued_auto_route_baseline(
    connection: &Connection,
    workspace_id: &str,
    turn_context: &mut ChatTurnPersistenceContext,
    provider_id: &mut Option<String>,
    model_id: &mut Option<String>,
) -> rusqlite::Result<QueuedAutoRouteIdentityRecord> {
    let (
        session_provider,
        session_model,
        baseline_provider,
        baseline_provider_type,
        baseline_model,
        baseline_reasoning,
        baseline_context_budget,
        baseline_provenance,
        route_generation,
    ) = connection.query_row(
        "SELECT sessions.provider_id, sessions.model_id,
                    config.local_provider_config_id, config.local_provider_type,
                    config.model_id, config.reasoning_depth, config.context_budget,
                    config.local_model_source,
                    config.local_route_generation
             FROM chat_sessions sessions
             LEFT JOIN active_session_configs config ON config.session_id = sessions.id
             WHERE sessions.id = ?1 AND sessions.workspace_id = ?2",
        params![turn_context.session_id, workspace_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i32>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            ))
        },
    )?;
    let baseline_provider = baseline_provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("dynamic"));
    let baseline_provider_type = baseline_provider_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("dynamic"));
    let baseline_model = baseline_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("dynamic"));
    let baseline_reasoning = baseline_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let baseline_provenance = baseline_provenance
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if !session_provider.eq_ignore_ascii_case("dynamic")
        || !session_model.eq_ignore_ascii_case("dynamic")
        || baseline_provider.is_none()
        || baseline_provider_type.is_none()
        || baseline_model.is_none()
        || baseline_reasoning.is_none()
        || baseline_context_budget.is_none_or(|value| value <= 0)
        || baseline_provenance.is_none()
        || route_generation <= 0
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "idle Auto-route messages require a complete immutable saved local identity before they are queued"
                .to_string(),
        ));
    }
    turn_context.provider_id = baseline_provider.unwrap().to_string();
    turn_context.model_id = baseline_model.unwrap().to_string();
    *provider_id = Some(turn_context.provider_id.clone());
    *model_id = Some(turn_context.model_id.clone());
    Ok(QueuedAutoRouteIdentityRecord {
        provider_config_id: turn_context.provider_id.clone(),
        provider_type: baseline_provider_type.unwrap().to_string(),
        model_id: turn_context.model_id.clone(),
        reasoning: baseline_reasoning.unwrap().to_string(),
        context_budget: baseline_context_budget.unwrap(),
        provenance: baseline_provenance.unwrap().to_string(),
        route_generation,
        frozen_at_ms: crate::foundation::clock::unix_time_ms_i64(),
    })
}

pub(super) fn emit_queued_auto_route_identity_receipt(
    turn_id: &str,
    session_id: &str,
    identity: &QueuedAutoRouteIdentityRecord,
) {
    crate::diagnostic_output::write_diagnostic_line(format_args!(
        "OOMU_NATIVE_RECEIPT {}",
        serde_json::json!({
            "kind": "auto_route_queued_identity_frozen",
            "receiptId": format!(
                "auto-route-queued-{}-{}",
                turn_id,
                identity.route_generation
            ),
            "sessionId": session_id,
            "turnId": turn_id,
            "providerConfigId": identity.provider_config_id,
            "providerType": identity.provider_type,
            "modelId": identity.model_id,
            "provenance": identity.provenance,
            "routeGeneration": identity.route_generation,
            "frozenAtMs": identity.frozen_at_ms,
            "committed": true,
            "rolledBack": false,
            "retryable": false,
        })
    ));
}
