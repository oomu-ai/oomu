use super::agent_execution_calendar_recovery::{
    narrow_step_amendment as narrow_calendar_step_amendment,
    receipt_context as calendar_recovery_receipt_context,
    resolved_arguments_sha256 as resolved_calendar_step_arguments_sha256,
    resolved_name as resolved_calendar_name, step_matches as calendar_step_matches,
};
use super::*;

mod cancellation;

#[derive(Debug, Clone)]
pub struct PlanExecutionCheckpoint {
    pub next_step_index: usize,
    pub completed_actions: Vec<(i64, String)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReceiptCompatibility {
    Explicit,
    ExternalReview,
    CheckpointReview,
    Legacy,
}

#[derive(Debug, Default)]
struct AgentActionRecoveryEvidence {
    action_count: usize,
    completed_count: usize,
    has_uncertain_effect: bool,
}

fn verified_completed_action_receipt(output: Option<&str>) -> bool {
    let Some(output) = output else {
        return false;
    };
    serde_json::from_str::<Value>(output).is_ok_and(|receipt| {
        receipt.get("status").and_then(Value::as_str) == Some("completed")
            && receipt.get("verified").and_then(Value::as_bool) == Some(true)
    })
}

fn action_recovery_row_is_safe(status: &str, output: Option<&str>) -> bool {
    if status == "completed" {
        return verified_completed_action_receipt(output);
    }
    if matches!(
        status,
        crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL
            | crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_READ_ONLY
    ) {
        return crate::agentic_loop::recovery::verified_unchanged_action_receipt(status, output);
    }
    crate::agentic_loop::recovery::automatic_replay_safe_action_status(status)
}

fn agent_action_recovery_evidence(
    connection: &Connection,
    plan_id: &str,
) -> rusqlite::Result<AgentActionRecoveryEvidence> {
    let mut statement = connection.prepare("SELECT status,output FROM actions WHERE plan_id=?1")?;
    let actions = statement
        .query_map(params![plan_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let completed_count = actions
        .iter()
        .filter(|(status, output)| {
            status == "completed" && verified_completed_action_receipt(output.as_deref())
        })
        .count();
    let checkpoint_index = connection
        .query_row(
            "SELECT current_step_index FROM plan_generation_states WHERE plan_id=?1",
            params![plan_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let checkpoint_matches = match checkpoint_index {
        Some(index) if index >= 0 => index as usize == completed_count,
        None => actions.is_empty(),
        _ => false,
    };
    Ok(AgentActionRecoveryEvidence {
        action_count: actions.len(),
        completed_count,
        has_uncertain_effect: actions
            .iter()
            .any(|(status, output)| !action_recovery_row_is_safe(status, output.as_deref()))
            || !checkpoint_matches,
    })
}

fn recovery_receipt_authorizes_new_plan(
    payload_json: &str,
    execution_id: &str,
    plan_id: &str,
) -> Option<ReceiptCompatibility> {
    let Ok(receipt) = serde_json::from_str::<Value>(payload_json) else {
        return None;
    };
    if receipt.get("schema").and_then(Value::as_str)
        != Some(crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA)
        || receipt.get("executionId").and_then(Value::as_str) != Some(execution_id)
        || receipt.get("planId").and_then(Value::as_str) != Some(plan_id)
        || receipt.get("recoverable").and_then(Value::as_bool) != Some(false)
    {
        return None;
    }
    match (
        receipt.get("recoveryAction").and_then(Value::as_str),
        receipt.get("changedState").and_then(Value::as_str),
    ) {
        (Some("start_new_plan"), Some("none" | "checkpoint_saved")) => {
            Some(ReceiptCompatibility::Explicit)
        }
        (Some("review_external_changes"), Some("external_changes")) => {
            Some(ReceiptCompatibility::ExternalReview)
        }
        (Some("review_external_changes"), Some("checkpoint_saved")) => {
            Some(ReceiptCompatibility::CheckpointReview)
        }
        (None, Some("none" | "checkpoint_saved")) => Some(ReceiptCompatibility::Legacy),
        _ => None,
    }
}

fn recovery_receipt_authorizes_resume(
    payload_json: &str,
    execution_id: &str,
    plan_id: &str,
) -> Option<ReceiptCompatibility> {
    let Ok(receipt) = serde_json::from_str::<Value>(payload_json) else {
        return None;
    };
    if receipt.get("schema").and_then(Value::as_str)
        == Some(crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA)
    {
        let matches_receipt = receipt.get("executionId").and_then(Value::as_str)
            == Some(execution_id)
            && receipt.get("planId").and_then(Value::as_str) == Some(plan_id)
            && receipt.get("recoverable").and_then(Value::as_bool) == Some(true)
            && matches!(
                receipt.get("changedState").and_then(Value::as_str),
                Some("none" | "checkpoint_saved")
            );
        if !matches_receipt {
            return None;
        }
        return match receipt.get("recoveryAction") {
            Some(Value::String(action)) if action == "resume_same_execution" => {
                Some(ReceiptCompatibility::Explicit)
            }
            None => Some(ReceiptCompatibility::Legacy),
            _ => None,
        };
    }
    (receipt.get("code").and_then(Value::as_str) == Some("agent_execution_interrupted")
        && receipt.get("recoverable").and_then(Value::as_bool) == Some(true))
    .then_some(ReceiptCompatibility::Legacy)
}

fn durable_replan_objective(
    context_json: &str,
    expected_plan_id: &str,
    expected_session_id: &str,
) -> Option<String> {
    let context = serde_json::from_str::<Value>(context_json).ok()?;
    if context.pointer("/plan/id").and_then(Value::as_str) != Some(expected_plan_id) {
        return None;
    }
    let durable_session_id = context
        .pointer("/turn_context/sessionId")
        .or_else(|| context.pointer("/turn_context/session_id"))
        .and_then(Value::as_str)?;
    if durable_session_id != expected_session_id {
        return None;
    }
    let objective = context.pointer("/plan/objective").and_then(Value::as_str)?;
    (!objective.trim().is_empty()).then(|| objective.to_string())
}

fn decision_pack_checkpoint_replan_is_idempotent(
    context_json: &str,
    completed_count: usize,
) -> bool {
    const OPERATIONS: [&str; 3] = [
        "create_decision_pack",
        "create_conflict_free_calendar_event",
        "draft_decision_pack_email",
    ];
    if completed_count == 0 || completed_count >= OPERATIONS.len() {
        return false;
    }
    let Ok(context) = serde_json::from_str::<Value>(context_json) else {
        return false;
    };
    let Some(steps) = context.pointer("/plan/steps").and_then(Value::as_array) else {
        return false;
    };
    steps.len() == OPERATIONS.len()
        && steps.iter().zip(OPERATIONS).all(|(step, expected)| {
            step.pointer("/tool/operation")
                .or_else(|| step.pointer("/tool/kind"))
                .and_then(Value::as_str)
                == Some(expected)
        })
}

impl PersistenceEngine {
    pub(crate) fn completed_agent_action_outputs_for_objective(
        &self,
        operation: &str,
        objective: &str,
    ) -> Result<Vec<String>, String> {
        let operation = operation.trim();
        let objective = objective.trim();
        if operation.is_empty() || objective.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT actions.output, executions.context_json
                 FROM actions
                 JOIN agent_executions executions
                   ON executions.plan_id=actions.plan_id
                 WHERE actions.tool=?1
                   AND actions.status='completed'
                   AND actions.output IS NOT NULL
                 ORDER BY actions.timestamp_ms DESC
                 LIMIT 32",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![operation], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        let mut outputs = Vec::new();
        for row in rows {
            let (output, context_json) = row.map_err(|error| error.to_string())?;
            let matches_objective = serde_json::from_str::<Value>(&context_json)
                .ok()
                .and_then(|context| {
                    context
                        .pointer("/plan/objective")
                        .and_then(Value::as_str)
                        .map(|candidate| candidate.trim() == objective)
                })
                .unwrap_or(false);
            if matches_objective && !outputs.contains(&output) {
                outputs.push(output);
            }
        }
        Ok(outputs)
    }

    pub async fn complete_agent_action_checkpoint(
        &self,
        action_id: i64,
        plan_id: String,
        expected_plan_json: String,
        next_step_index: usize,
        output: String,
        generated_text: String,
    ) -> Result<(), String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            engine.commit_completed_agent_action_checkpoint(
                action_id,
                &plan_id,
                &expected_plan_json,
                next_step_index,
                &output,
                &generated_text,
            )
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    pub async fn mark_agent_action_invocation_started(
        &self,
        action_id: i64,
        prepared_status: String,
        started_status: String,
    ) -> Result<(), String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _guard = engine.lock_writes();
            let connection = engine.open_connection()?;
            let changed = connection.execute(
                "UPDATE actions SET status=?1,timestamp_ms=?2
                 WHERE id=?3 AND status=?4 AND output IS NULL",
                params![started_status, unix_time_ms(), action_id, prepared_status],
            )?;
            if changed != 1 {
                return Err(rusqlite::Error::InvalidParameterName(
                    "agent action lost its durable pre-invocation boundary".to_string(),
                ));
            }
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    pub async fn record_agent_action_invocation_result(
        &self,
        action_id: i64,
        expected_status: String,
        output: String,
        result_status: String,
    ) -> Result<(), String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _guard = engine.lock_writes();
            let connection = engine.open_connection()?;
            let changed = connection.execute(
                "UPDATE actions SET output=?1,status=?2,timestamp_ms=?3
                 WHERE id=?4 AND status=?5 AND output IS NULL",
                params![
                    output,
                    result_status,
                    unix_time_ms(),
                    action_id,
                    expected_status
                ],
            )?;
            if changed != 1 {
                return Err(rusqlite::Error::InvalidParameterName(
                    "agent action lost its durable invocation result boundary".to_string(),
                ));
            }
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    pub fn has_uncertain_agent_action_effect(&self, plan_id: &str) -> Result<bool, String> {
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        agent_action_recovery_evidence(&connection, plan_id)
            .map(|evidence| evidence.has_uncertain_effect)
            .map_err(|error| error.to_string())
    }

    pub(super) fn commit_completed_agent_action_checkpoint(
        &self,
        action_id: i64,
        plan_id: &str,
        expected_plan_json: &str,
        next_step_index: usize,
        output: &str,
        generated_text: &str,
    ) -> rusqlite::Result<()> {
        let plan_id = plan_id.trim();
        if action_id <= 0
            || plan_id.is_empty()
            || expected_plan_json.trim().is_empty()
            || next_step_index == 0
            || output.trim().is_empty()
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "completed action checkpoint requires an action, signed plan, cursor, and receipt"
                    .to_string(),
            ));
        }
        let output_value = serde_json::from_str::<Value>(output).map_err(json_to_sql_error)?;
        if output_value.get("status").and_then(Value::as_str) != Some("completed")
            || output_value.get("verified").and_then(Value::as_bool) != Some(true)
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "completed action checkpoint requires a verified completed receipt".to_string(),
            ));
        }
        let prior_step_index = next_step_index - 1;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let state = transaction
            .query_row(
                "SELECT plan_json,current_step_index,status
                 FROM plan_generation_states WHERE plan_id=?1",
                params![plan_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((stored_plan_json, stored_step_index, state_status)) = state else {
            return Err(rusqlite::Error::InvalidParameterName(
                "completed action checkpoint is missing its signed plan state".to_string(),
            ));
        };
        if stored_plan_json != expected_plan_json
            || stored_step_index != prior_step_index as i64
            || !matches!(state_status.as_str(), "running" | "checkpointed")
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "completed action checkpoint does not match the signed plan cursor".to_string(),
            ));
        }
        let completed_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM actions
             WHERE plan_id=?1 AND status='completed' AND output IS NOT NULL",
            params![plan_id],
            |row| row.get(0),
        )?;
        if completed_count != prior_step_index as i64 {
            return Err(rusqlite::Error::InvalidParameterName(
                "completed action checkpoint has an inconsistent receipt count".to_string(),
            ));
        }
        let action_matches: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM actions
             WHERE id=?1 AND plan_id=?2
               AND status IN (?3,?4) AND output IS NULL",
            params![
                action_id,
                plan_id,
                crate::agentic_loop::recovery::ACTION_STARTED_EFFECTFUL,
                crate::agentic_loop::recovery::ACTION_STARTED_READ_ONLY,
            ],
            |row| row.get(0),
        )?;
        if action_matches != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "completed action checkpoint lost its running action boundary".to_string(),
            ));
        }
        let now = unix_time_ms();
        let action_changed = transaction.execute(
            "UPDATE actions SET output=?1,status='completed',timestamp_ms=?2
             WHERE id=?3 AND plan_id=?4
               AND status IN (?5,?6) AND output IS NULL",
            params![
                output,
                now,
                action_id,
                plan_id,
                crate::agentic_loop::recovery::ACTION_STARTED_EFFECTFUL,
                crate::agentic_loop::recovery::ACTION_STARTED_READ_ONLY,
            ],
        )?;
        if action_changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "completed action checkpoint could not seal its action receipt".to_string(),
            ));
        }
        let state_changed = transaction.execute(
            "UPDATE plan_generation_states
             SET current_step_index=?1,status='checkpointed',generated_text=?2,timestamp_ms=?3
             WHERE plan_id=?4 AND plan_json=?5 AND current_step_index=?6
               AND status IN ('running','checkpointed')",
            params![
                next_step_index as i64,
                generated_text,
                now,
                plan_id,
                expected_plan_json,
                prior_step_index as i64,
            ],
        )?;
        if state_changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "completed action checkpoint could not advance its signed plan cursor".to_string(),
            ));
        }
        transaction.commit()
    }

    pub fn canonical_agent_execution_origin_context(
        &self,
        requested: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<ChatTurnPersistenceContext> {
        validate_chat_turn_context_fields(requested)?;
        let connection = self.open_connection()?;
        connection.query_row(
            "
            SELECT turns.turn_id, turns.generation_token, turns.session_id, turns.agent_id,
                   turns.provider_id, turns.model_id, turns.parent_turn_id,
                   turns.root_turn_id, turns.turn_kind
            FROM chat_turns turns
            JOIN chat_sessions sessions
              ON sessions.id = turns.session_id
             AND sessions.workspace_id = turns.workspace_id
            WHERE turns.turn_id = ?1
              AND turns.generation_token = ?2
              AND turns.session_id = ?3
              AND turns.agent_id = ?4
              AND turns.root_turn_id = ?5
              AND turns.turn_kind = ?6
              AND COALESCE(turns.parent_turn_id, '') = COALESCE(?7, '')
              AND turns.status IN ('running', 'completed', 'escalated')
              AND sessions.agent_id = turns.agent_id
              AND (
                    (turns.provider_id = ?8 AND turns.model_id = ?9)
                 OR (
                        lower(?8) = 'dynamic'
                    AND lower(?9) = 'dynamic'
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
            ",
            params![
                requested.turn_id,
                requested.generation_token,
                requested.session_id,
                requested.agent_id,
                requested.root_turn_id,
                requested.turn_kind,
                requested.parent_turn_id,
                requested.provider_id,
                requested.model_id,
            ],
            |row| {
                Ok(ChatTurnPersistenceContext {
                    turn_id: row.get(0)?,
                    generation_token: row.get(1)?,
                    session_id: row.get(2)?,
                    agent_id: row.get(3)?,
                    provider_id: row.get(4)?,
                    model_id: row.get(5)?,
                    parent_turn_id: row.get(6)?,
                    root_turn_id: row.get(7)?,
                    turn_kind: row.get(8)?,
                })
            },
        )
    }

    pub fn begin_agent_execution(
        &self,
        execution_id: &str,
        plan_id: &str,
        context: &ChatTurnPersistenceContext,
        context_json: &str,
    ) -> rusqlite::Result<()> {
        validate_chat_turn_context_fields(context)?;
        let execution_id = execution_id.trim();
        let plan_id = plan_id.trim();
        if execution_id.is_empty() || plan_id.is_empty() || context_json.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution requires an ID, plan, and immutable context".to_string(),
            ));
        }
        serde_json::from_str::<Value>(context_json).map_err(json_to_sql_error)?;

        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let authority_matches: i64 = transaction.query_row(
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
              AND turns.provider_id = ?5
              AND turns.model_id = ?6
              AND turns.root_turn_id = ?7
              AND turns.turn_kind = ?8
              AND COALESCE(turns.parent_turn_id, '') = COALESCE(?9, '')
              AND turns.status IN ('running', 'completed', 'escalated')
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
        if authority_matches != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution origin is stale, cancelled, or deleted".to_string(),
            ));
        }
        let existing_execution: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM agent_executions
             WHERE plan_id = ?1 AND turn_id = ?2 AND generation_token = ?3",
            params![plan_id, context.turn_id, context.generation_token],
            |row| row.get(0),
        )?;
        if existing_execution != 0 {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution already exists for this plan origin".to_string(),
            ));
        }
        let project_id = match transaction.query_row(
            "SELECT project_id FROM chat_sessions WHERE id=?1",
            params![context.session_id],
            |row| row.get::<_, Option<String>>(0),
        )? {
            Some(project_id) => project_id,
            None => crate::projects::repository::ensure_internal_local_files_project(&transaction)?,
        };
        let now = unix_time_ms();
        transaction.execute(
            "
            INSERT INTO agent_executions (
                execution_id, plan_id, session_id, project_id, agent_id, provider_id, model_id,
                turn_id, generation_token, parent_turn_id, root_turn_id, turn_kind,
                context_json, status, created_at_ms, updated_at_ms, encryption_state
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'running', ?14, ?14, ?15)
            ",
            params![
                execution_id,
                plan_id,
                context.session_id,
                project_id,
                context.agent_id,
                context.provider_id,
                context.model_id,
                context.turn_id,
                context.generation_token,
                context.parent_turn_id,
                context.root_turn_id,
                context.turn_kind,
                context_json,
                now,
                get_current_encryption_state(),
            ],
        )?;
        super::agent_execution_lifecycle::ensure_agent_task_run(
            &transaction,
            execution_id,
            &project_id,
            &context.agent_id,
            now,
        )?;
        transaction.execute(
            "
            INSERT INTO agent_execution_logs (
                execution_id, plan_id, session_id, agent_id, level, phase,
                message, payload_json, created_at_ms, encryption_state
            )
            VALUES (?1, ?2, ?3, ?4, 'info', 'queued',
                    'Execution accepted with immutable turn ownership.', NULL, ?5, ?6)
            ",
            params![
                execution_id,
                plan_id,
                context.session_id,
                context.agent_id,
                now,
                get_current_encryption_state(),
            ],
        )?;
        transaction.commit()
    }

    pub fn load_plan_execution_checkpoint(
        &self,
        plan_id: &str,
        expected_plan_json: &str,
        step_count: usize,
    ) -> Result<Option<PlanExecutionCheckpoint>, String> {
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let state = connection
            .query_row(
                "SELECT plan_json, current_step_index FROM plan_generation_states WHERE plan_id = ?1",
                params![plan_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((stored_plan_json, current_step_index)) = state else {
            return Ok(None);
        };
        if stored_plan_json != expected_plan_json
            || current_step_index < 0
            || current_step_index as usize > step_count
        {
            return Err("execution_checkpoint_plan_mismatch".to_string());
        }
        let next_step_index = current_step_index as usize;
        let mut statement = connection
            .prepare(
                "SELECT id, output FROM actions
                 WHERE plan_id = ?1 AND status = 'completed' AND output IS NOT NULL
                 ORDER BY id ASC",
            )
            .map_err(|error| error.to_string())?;
        let completed_actions = statement
            .query_map(params![plan_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        if completed_actions.len() != next_step_index {
            return Err("execution_checkpoint_action_mismatch".to_string());
        }
        Ok(Some(PlanExecutionCheckpoint {
            next_step_index,
            completed_actions,
        }))
    }

    pub fn load_resumable_agent_execution_request(
        &self,
        execution_id: &str,
    ) -> rusqlite::Result<String> {
        let connection = self.open_connection()?;
        let execution_id = execution_id.trim();
        let (plan_id, context_json, recovery_phase, payload_json) = connection.query_row(
            "SELECT executions.plan_id,executions.context_json,
                    (SELECT logs.phase FROM agent_execution_logs logs
                     WHERE logs.execution_id=executions.execution_id
                       AND logs.payload_json IS NOT NULL
                     ORDER BY logs.id DESC LIMIT 1),
                    (SELECT logs.payload_json FROM agent_execution_logs logs
                     WHERE logs.execution_id=executions.execution_id
                       AND logs.payload_json IS NOT NULL
                     ORDER BY logs.id DESC LIMIT 1)
             FROM agent_executions executions
             WHERE executions.execution_id=?1 AND executions.status='halted'",
            params![execution_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )?;
        let evidence = agent_action_recovery_evidence(&connection, &plan_id)?;
        let compatibility = payload_json.as_deref().and_then(|payload| {
            recovery_receipt_authorizes_resume(payload, execution_id, &plan_id)
        });
        let restart_binding_matches = recovery_phase.as_deref() != Some("restart_recovery_ready")
            || payload_json.as_deref().is_some_and(|payload| {
                super::agent_execution_restart::restart_receipt_matches_pending_action(
                    self,
                    execution_id,
                    &plan_id,
                    &context_json,
                    payload,
                )
            });
        if evidence.has_uncertain_effect
            || compatibility.is_none()
            || !restart_binding_matches
            || (compatibility == Some(ReceiptCompatibility::Legacy)
                && evidence.action_count != evidence.completed_count)
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution has an unresolved action effect and is not resumable".to_string(),
            ));
        }
        Ok(context_json)
    }

    pub fn load_calendar_recovery_execution_request(
        &self,
        execution_id: &str,
        session_id: &str,
    ) -> rusqlite::Result<(String, usize, String, Vec<String>)> {
        let connection = self.open_connection()?;
        let execution_id = execution_id.trim();
        let session_id = session_id.trim();
        let (plan_id, context_json, plan_json, current_step_index, payload_json) = connection
            .query_row(
                "SELECT executions.plan_id,executions.context_json,state.plan_json,
                        state.current_step_index,
                        (SELECT logs.payload_json FROM agent_execution_logs logs
                         WHERE logs.execution_id=executions.execution_id
                           AND logs.payload_json IS NOT NULL
                         ORDER BY logs.id DESC LIMIT 1)
                 FROM agent_executions executions
                 JOIN plan_generation_states state ON state.plan_id=executions.plan_id
                 WHERE executions.execution_id=?1 AND executions.session_id=?2
                   AND executions.status='halted'",
                params![execution_id, session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?;
        if current_step_index < 0 {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery checkpoint is invalid".to_string(),
            ));
        }
        let evidence = agent_action_recovery_evidence(&connection, &plan_id)?;
        let step_index = current_step_index as usize;
        if evidence.has_uncertain_effect || evidence.completed_count != step_index {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery has unresolved action evidence".to_string(),
            ));
        }
        let Some(recovery_context) =
            calendar_recovery_receipt_context(&payload_json, execution_id, &plan_id)
        else {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery receipt is unavailable".to_string(),
            ));
        };
        let durable_request =
            serde_json::from_str::<crate::agentic_loop::AgentPlanExecutionRequest>(&context_json)
                .map_err(|_| {
                rusqlite::Error::InvalidParameterName(
                    "calendar recovery execution context is invalid".to_string(),
                )
            })?;
        let durable_plan_json = serde_json::to_string(&durable_request.plan).map_err(|_| {
            rusqlite::Error::InvalidParameterName(
                "calendar recovery signed plan is invalid".to_string(),
            )
        })?;
        if durable_plan_json != plan_json
            || durable_request.plan.id != plan_id
            || !calendar_step_matches(
                &durable_request,
                step_index,
                &recovery_context.requested_calendar_name,
            )
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery does not match the paused signed step".to_string(),
            ));
        }
        if let Some(expected_digest) = recovery_context.denied_arguments_sha256.as_deref() {
            let actual_digest = resolved_calendar_step_arguments_sha256(
                self,
                &connection,
                execution_id,
                &plan_id,
                &durable_request,
                step_index,
            )
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(
                    "calendar recovery could not reconstruct the denied action".to_string(),
                )
            })?;
            if actual_digest != expected_digest {
                return Err(rusqlite::Error::InvalidParameterName(
                    "calendar recovery denied-action binding does not match".to_string(),
                ));
            }
        }
        Ok((
            context_json,
            step_index,
            recovery_context.requested_calendar_name,
            recovery_context.available_calendar_names,
        ))
    }

    pub fn commit_agent_calendar_recovery_resolution(
        &self,
        execution_id: &str,
        session_id: &str,
        expected_context_json: &str,
        resolved_context_json: &str,
        resolved_plan_json: &str,
        resolution_payload_json: &str,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let (plan_id, stored_context_json, stored_plan_json, step_index, receipt_json) =
            transaction.query_row(
                "SELECT executions.plan_id,executions.context_json,state.plan_json,
                        state.current_step_index,
                        (SELECT logs.payload_json FROM agent_execution_logs logs
                         WHERE logs.execution_id=executions.execution_id
                           AND logs.payload_json IS NOT NULL
                         ORDER BY logs.id DESC LIMIT 1)
                 FROM agent_executions executions
                 JOIN plan_generation_states state ON state.plan_id=executions.plan_id
                 WHERE executions.execution_id=?1 AND executions.session_id=?2
                   AND executions.status='halted'",
                params![execution_id, session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?;
        let Some(recovery_context) =
            calendar_recovery_receipt_context(&receipt_json, execution_id, &plan_id)
        else {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery is no longer awaiting a target".to_string(),
            ));
        };
        let requested = recovery_context.requested_calendar_name.as_str();
        let Some(selected) =
            resolved_calendar_name(resolution_payload_json, execution_id, &plan_id, requested)
        else {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery resolution receipt is invalid".to_string(),
            ));
        };
        if stored_context_json != expected_context_json || step_index < 0 {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery context changed before resolution".to_string(),
            ));
        }
        let old_request = serde_json::from_str::<crate::agentic_loop::AgentPlanExecutionRequest>(
            &stored_context_json,
        )
        .map_err(|_| rusqlite::Error::InvalidParameterName("invalid paused plan".to_string()))?;
        let new_request = serde_json::from_str::<crate::agentic_loop::AgentPlanExecutionRequest>(
            resolved_context_json,
        )
        .map_err(|_| rusqlite::Error::InvalidParameterName("invalid resolved plan".to_string()))?;
        let index = step_index as usize;
        let evidence = agent_action_recovery_evidence(&transaction, &plan_id)?;
        let denied_action_digest_matches = recovery_context
            .denied_arguments_sha256
            .as_deref()
            .map(|expected_digest| {
                resolved_calendar_step_arguments_sha256(
                    self,
                    &transaction,
                    execution_id,
                    &plan_id,
                    &old_request,
                    index,
                )
                .is_some_and(|actual_digest| actual_digest == expected_digest)
            })
            .unwrap_or(true);
        if old_request.plan.id != plan_id
            || new_request.plan.id != plan_id
            || serde_json::to_string(&old_request.plan).ok().as_deref()
                != Some(stored_plan_json.as_str())
            || serde_json::to_string(&new_request.plan).ok().as_deref() != Some(resolved_plan_json)
            || evidence.has_uncertain_effect
            || evidence.completed_count != index
            || !denied_action_digest_matches
            || !calendar_step_matches(&old_request, index, requested)
            || old_request.plan.steps.len() != new_request.plan.steps.len()
            || serde_json::to_value(&old_request.plan.steps[..index]).ok()
                != serde_json::to_value(&new_request.plan.steps[..index]).ok()
            || serde_json::to_value(&old_request.plan.steps[index + 1..]).ok()
                != serde_json::to_value(&new_request.plan.steps[index + 1..]).ok()
            || !narrow_calendar_step_amendment(&old_request, &new_request, index, &selected)
            || new_request.authority_proof_id.is_some()
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery amendment changed work outside the paused step".to_string(),
            ));
        }
        let now = unix_time_ms();
        let execution_changed = transaction.execute(
            "UPDATE agent_executions SET context_json=?1,updated_at_ms=?2
             WHERE execution_id=?3 AND context_json=?4 AND status='halted'",
            params![
                resolved_context_json,
                now,
                execution_id,
                expected_context_json
            ],
        )?;
        if execution_changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery execution compare-and-swap failed".to_string(),
            ));
        }
        let plan_changed = transaction.execute(
            "UPDATE plan_generation_states SET plan_json=?1,timestamp_ms=?2
             WHERE plan_id=?3 AND plan_json=?4 AND current_step_index=?5",
            params![
                resolved_plan_json,
                now,
                plan_id,
                stored_plan_json,
                step_index
            ],
        )?;
        if plan_changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery plan compare-and-swap failed".to_string(),
            ));
        }
        let denial_message_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM chat_messages
             WHERE session_id=?1 AND role='assistant' AND content=?2 AND metadata_json=?2",
            params![session_id, receipt_json],
            |row| row.get(0),
        )?;
        if denial_message_count != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery message compare-and-swap failed".to_string(),
            ));
        }
        let resolution_message_inserted = transaction.execute(
            "INSERT INTO chat_messages (
                workspace_id,session_id,agent_id,role,content,provider_id,model_id,
                metadata_json,is_compacted,compaction_type,timestamp_ms,encryption_state
             ) SELECT sessions.workspace_id,executions.session_id,executions.agent_id,
                      'assistant',?1,executions.provider_id,executions.model_id,
                      ?1,0,'raw',?2,?3
               FROM agent_executions executions
               JOIN chat_sessions sessions ON sessions.id=executions.session_id
               WHERE executions.execution_id=?4 AND executions.session_id=?5
                 AND executions.status='halted'",
            params![
                resolution_payload_json,
                now,
                get_current_encryption_state(),
                execution_id,
                session_id,
            ],
        )?;
        if resolution_message_inserted != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery resolution message insert failed".to_string(),
            ));
        }
        transaction.execute(
            "INSERT INTO agent_execution_logs (
                execution_id,plan_id,session_id,agent_id,level,phase,message,
                payload_json,created_at_ms,encryption_state
             ) SELECT execution_id,plan_id,session_id,agent_id,'info','calendar_target_resolved',
                      'Calendar target resolved by the user.',?1,?2,?3
               FROM agent_executions WHERE execution_id=?4",
            params![
                resolution_payload_json,
                now,
                get_current_encryption_state(),
                execution_id,
            ],
        )?;
        transaction.commit()
    }

    pub fn cancel_agent_calendar_recovery(
        &self,
        execution_id: &str,
        session_id: &str,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let (plan_id, step_index, payload_json): (String, i64, String) = transaction.query_row(
            "SELECT executions.plan_id,state.current_step_index,
                    (SELECT logs.payload_json FROM agent_execution_logs logs
                     WHERE logs.execution_id=executions.execution_id
                       AND logs.payload_json IS NOT NULL
                     ORDER BY logs.id DESC LIMIT 1)
             FROM agent_executions executions
             JOIN plan_generation_states state ON state.plan_id=executions.plan_id
             WHERE executions.execution_id=?1 AND executions.session_id=?2
               AND executions.status='halted'",
            params![execution_id, session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let Some(recovery_context) =
            calendar_recovery_receipt_context(&payload_json, execution_id, &plan_id)
        else {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery is no longer awaiting a target".to_string(),
            ));
        };
        let requested = recovery_context.requested_calendar_name;
        let cancellation_payload = serde_json::json!({
            "schema": crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
            "executionId": execution_id,
            "planId": plan_id,
            "code": "calendar_recovery_cancelled",
            "boundary": "CalendarRecovery",
            "recoverable": false,
            "recoveryAction": "calendar_recovery_cancelled",
            "message": "The paused calendar task was cancelled by the user.",
            "context": {
                "requestedCalendarName": requested,
                "cancelled": true,
            },
            "changedState": if step_index > 0 { "checkpoint_saved" } else { "none" },
        })
        .to_string();
        let now = unix_time_ms();
        let changed = transaction.execute(
            "UPDATE agent_executions SET status='cancelled',updated_at_ms=?1
             WHERE execution_id=?2 AND status='halted'",
            params![now, execution_id],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery could not be cancelled".to_string(),
            ));
        }
        let message_changed = transaction.execute(
            "UPDATE chat_messages SET content=?1,metadata_json=?1,timestamp_ms=?2
             WHERE id=(
                 SELECT id FROM chat_messages
                 WHERE session_id=?3 AND content=?4 AND metadata_json=?4
                 ORDER BY id DESC LIMIT 1
             )",
            params![cancellation_payload, now, session_id, payload_json],
        )?;
        if message_changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "calendar recovery cancellation message compare-and-swap failed".to_string(),
            ));
        }
        transaction.execute(
            "UPDATE chat_turns SET status='completed',completed_at_ms=?1
             WHERE (turn_id,generation_token)=(
                 SELECT turn_id,generation_token FROM agent_executions WHERE execution_id=?2
             ) AND status='escalated'",
            params![now, execution_id],
        )?;
        transaction.execute(
            "UPDATE task_runs SET state='cancelled',last_error='Cancelled by user.',
                    completed_at_ms=?1,updated_at_ms=?1,recovery_state='reconciled'
             WHERE runtime_kind='agent' AND runtime_record_id=?2",
            params![now, execution_id],
        )?;
        transaction.execute(
            "UPDATE plan_generation_states SET status='cancelled',
                    generated_text='Cancelled by user.',timestamp_ms=?1 WHERE plan_id=?2",
            params![now, plan_id],
        )?;
        transaction.execute(
            "INSERT INTO agent_execution_logs (
                execution_id,plan_id,session_id,agent_id,level,phase,message,
                payload_json,created_at_ms,encryption_state
             ) SELECT execution_id,plan_id,session_id,agent_id,'info','cancelled',
                      'Calendar recovery was cancelled by the user.',?1,?2,?3
               FROM agent_executions WHERE execution_id=?4",
            params![
                cancellation_payload,
                now,
                get_current_encryption_state(),
                execution_id,
            ],
        )?;
        transaction.commit()
    }

    pub fn prepare_agent_execution_replan(
        &self,
        execution_id: &str,
        session_id: &str,
    ) -> rusqlite::Result<Option<String>> {
        let execution_id = execution_id.trim();
        let session_id = session_id.trim();
        if execution_id.is_empty() || session_id.is_empty() {
            return Ok(None);
        }
        let connection = self.open_connection()?;
        let record = connection
            .query_row(
                "SELECT executions.plan_id, executions.context_json,
                        (SELECT logs.payload_json
                         FROM agent_execution_logs logs
                         WHERE logs.execution_id=executions.execution_id
                           AND logs.payload_json IS NOT NULL
                           AND logs.phase='failed'
                         ORDER BY logs.id DESC LIMIT 1)
                 FROM agent_executions executions
                 JOIN chat_turns turns
                   ON turns.turn_id=executions.turn_id
                  AND turns.generation_token=executions.generation_token
                  AND turns.session_id=executions.session_id
                 JOIN chat_sessions sessions
                   ON sessions.id=turns.session_id
                  AND sessions.workspace_id=turns.workspace_id
                  AND sessions.agent_id=executions.agent_id
                 WHERE executions.execution_id=?1
                   AND executions.session_id=?2
                   AND executions.status='failed'",
                params![execution_id, session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((plan_id, context_json, Some(payload_json))) = record else {
            return Ok(None);
        };
        let Some(compatibility) =
            recovery_receipt_authorizes_new_plan(&payload_json, execution_id, &plan_id)
        else {
            return Ok(None);
        };
        let evidence = agent_action_recovery_evidence(&connection, &plan_id)?;
        let safe_to_replan = match compatibility {
            ReceiptCompatibility::Explicit => {
                evidence.completed_count == 0 && !evidence.has_uncertain_effect
            }
            ReceiptCompatibility::ExternalReview => evidence.completed_count == 0,
            ReceiptCompatibility::CheckpointReview => {
                !evidence.has_uncertain_effect
                    && decision_pack_checkpoint_replan_is_idempotent(
                        &context_json,
                        evidence.completed_count,
                    )
            }
            ReceiptCompatibility::Legacy => {
                evidence.completed_count == 0
                    && evidence.action_count == 0
                    && !evidence.has_uncertain_effect
            }
        };
        if !safe_to_replan {
            return Ok(None);
        }
        Ok(durable_replan_objective(
            &context_json,
            &plan_id,
            session_id,
        ))
    }

    pub fn resume_agent_execution(
        &self,
        execution_id: &str,
        plan_id: &str,
        context: &ChatTurnPersistenceContext,
        context_json: &str,
    ) -> rusqlite::Result<i64> {
        validate_chat_turn_context_fields(context)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let (recovery_phase, payload_json) = transaction
            .query_row(
                "SELECT phase,payload_json FROM agent_execution_logs
                 WHERE execution_id=?1 AND payload_json IS NOT NULL
                 ORDER BY id DESC LIMIT 1",
                params![execution_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .map_or((None, None), |(phase, payload)| {
                (Some(phase), Some(payload))
            });
        let evidence = agent_action_recovery_evidence(&transaction, plan_id)?;
        let compatibility = payload_json
            .as_deref()
            .and_then(|payload| recovery_receipt_authorizes_resume(payload, execution_id, plan_id));
        let restart_binding_matches = recovery_phase.as_deref()
            != Some("restart_recovery_ready")
            || payload_json.as_deref().is_some_and(|payload| {
                super::agent_execution_restart::restart_receipt_matches_pending_action_in_connection(
                    self,
                    &transaction,
                    execution_id,
                    plan_id,
                    context_json,
                    payload,
                )
            });
        if evidence.has_uncertain_effect
            || compatibility.is_none()
            || !restart_binding_matches
            || (compatibility == Some(ReceiptCompatibility::Legacy)
                && evidence.action_count != evidence.completed_count)
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution has an unresolved action effect and cannot resume".to_string(),
            ));
        }
        let now = unix_time_ms();
        let changed = transaction.execute(
            "UPDATE agent_executions
             SET status = 'running', updated_at_ms = ?1
             WHERE execution_id = ?2 AND plan_id = ?3 AND context_json = ?4
               AND turn_id = ?5 AND generation_token = ?6 AND status = 'halted'",
            params![
                now,
                execution_id,
                plan_id,
                context_json,
                context.turn_id,
                context.generation_token,
            ],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution is not at a resumable boundary".to_string(),
            ));
        }
        let origin_matches: i64 = transaction.query_row(
            "SELECT COUNT(*)
             FROM chat_turns turns
             JOIN chat_sessions sessions
               ON sessions.id = turns.session_id
              AND sessions.workspace_id = turns.workspace_id
             WHERE turns.turn_id = ?1
               AND turns.generation_token = ?2
               AND turns.session_id = ?3
               AND turns.agent_id = ?4
               AND turns.provider_id = ?5
               AND turns.model_id = ?6
               AND turns.root_turn_id = ?7
               AND turns.turn_kind = ?8
               AND COALESCE(turns.parent_turn_id, '') = COALESCE(?9, '')
               AND turns.status IN ('completed', 'escalated')
               AND sessions.agent_id = turns.agent_id",
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
        if origin_matches != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution originating turn is not resumable".to_string(),
            ));
        }
        let stream_start_after_log_id = transaction.query_row(
            "SELECT COALESCE(MAX(id),0) FROM agent_execution_logs WHERE execution_id=?1",
            params![execution_id],
            |row| row.get::<_, i64>(0),
        )?;
        transaction.execute(
            "UPDATE chat_turns SET status='running',completed_at_ms=NULL
             WHERE turn_id=?1 AND generation_token=?2 AND status IN ('completed','escalated')",
            params![context.turn_id, context.generation_token],
        )?;
        transaction.execute(
            "UPDATE plan_generation_states SET status='running',
                    generated_text='Resuming from the verified checkpoint.',timestamp_ms=?1
             WHERE plan_id=?2",
            params![now, plan_id],
        )?;
        transaction.execute(
            "UPDATE task_runs SET state='running',last_error=NULL,updated_at_ms=?1,
                    completed_at_ms=NULL,recovery_state='reconciled'
             WHERE runtime_kind='agent' AND runtime_record_id=?2",
            params![now, execution_id],
        )?;
        transaction.execute(
            "INSERT INTO agent_execution_logs (
                execution_id, plan_id, session_id, agent_id, level, phase,
                message, payload_json, created_at_ms, encryption_state
             ) VALUES (?1, ?2, ?3, ?4, 'info', 'resumed',
                       'Execution resumed from its verified checkpoint.', NULL, ?5, ?6)",
            params![
                execution_id,
                plan_id,
                context.session_id,
                context.agent_id,
                now,
                get_current_encryption_state(),
            ],
        )?;
        transaction.commit()?;
        Ok(stream_start_after_log_id)
    }

    pub fn validate_agent_execution_origin(
        &self,
        execution_id: &str,
        plan_id: &str,
        context: &ChatTurnPersistenceContext,
        context_json: &str,
    ) -> rusqlite::Result<()> {
        validate_chat_turn_context_fields(context)?;
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let matches: i64 = connection.query_row(
            "
            SELECT COUNT(*)
            FROM agent_executions executions
            JOIN chat_turns turns
              ON turns.turn_id = executions.turn_id
             AND turns.generation_token = executions.generation_token
            JOIN chat_sessions sessions
              ON sessions.id = turns.session_id
             AND sessions.workspace_id = turns.workspace_id
            WHERE executions.execution_id = ?1
              AND executions.plan_id = ?2
              AND executions.context_json = ?3
              AND executions.status = 'running'
              AND turns.turn_id = ?4
              AND turns.generation_token = ?5
              AND turns.session_id = ?6
              AND turns.agent_id = ?7
              AND turns.provider_id = ?8
              AND turns.model_id = ?9
              AND turns.root_turn_id = ?10
              AND turns.turn_kind = ?11
              AND COALESCE(turns.parent_turn_id, '') = COALESCE(?12, '')
              AND turns.status IN ('running', 'completed', 'escalated')
              AND sessions.agent_id = turns.agent_id
            ",
            params![
                execution_id,
                plan_id,
                context_json,
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
        if matches != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution origin is stale, cancelled, or deleted".to_string(),
            ));
        }
        Ok(())
    }
}
