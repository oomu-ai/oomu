use super::*;

pub(super) fn is_terminal_phase(phase: &str) -> bool {
    matches!(
        phase,
        "completed" | "failed" | "halted" | "restart_recovery_ready"
    )
}

pub(super) fn audit_recovery(engine: &PersistenceEngine) {
    if let Err(error) = engine.mark_interrupted_actions() {
        eprintln!(
            "SOVEREIGN_RECOVERY_AUDIT_FAILED {}",
            crate::redaction::redacted_log_text(&error.to_string())
        );
    }
}

#[derive(Debug)]
struct InterruptedApprovedExecution {
    execution_id: String,
    plan_id: String,
    context: ChatTurnPersistenceContext,
    context_json: String,
    plan_json: String,
    next_step_index: usize,
}

#[derive(Debug)]
struct InterruptedApprovedExecutionRecovery {
    execution_id: String,
    plan_id: String,
    context: ChatTurnPersistenceContext,
    context_json: String,
    message: String,
    receipt_json: String,
}

#[derive(Debug)]
struct DurableRestartAction {
    id: i64,
    operation: String,
    input: String,
    output: Option<String>,
    status: String,
}

fn durable_restart_actions(
    connection: &Connection,
    plan_id: &str,
) -> rusqlite::Result<Vec<DurableRestartAction>> {
    let mut statement = connection.prepare(
        "SELECT id,tool,input,output,status FROM actions WHERE plan_id=?1 ORDER BY id ASC",
    )?;
    let actions = statement
        .query_map(params![plan_id], |row| {
            Ok(DurableRestartAction {
                id: row.get(0)?,
                operation: row.get(1)?,
                input: row.get(2)?,
                output: row.get(3)?,
                status: row.get(4)?,
            })
        })?
        .collect();
    actions
}

fn replay_safe_action_evidence(action: &DurableRestartAction) -> Option<Value> {
    if action.status == "completed" {
        return None;
    }
    let receipt_sha256 = if matches!(
        action.status.as_str(),
        crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL
            | crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_READ_ONLY
    ) {
        let output = action.output.as_deref()?;
        if !crate::agentic_loop::recovery::verified_unchanged_action_receipt(
            &action.status,
            Some(output),
        ) || serde_json::from_str::<Value>(output)
            .ok()?
            .get("operation")
            .and_then(Value::as_str)
            != Some(action.operation.as_str())
        {
            return None;
        }
        Some(crate::foundation::digest::sha256_hex(output.as_bytes()))
    } else if crate::agentic_loop::recovery::automatic_replay_safe_action_status(&action.status)
        && action.output.is_none()
    {
        None
    } else {
        return None;
    };
    Some(serde_json::json!({
        "actionId": action.id,
        "operation": action.operation,
        "status": action.status,
        "inputSha256": crate::foundation::digest::sha256_hex(action.input.as_bytes()),
        "receiptSha256": receipt_sha256,
    }))
}

fn replay_safe_action_evidence_for_checkpoint(
    actions: &[DurableRestartAction],
    completed_action_ids: &[i64],
) -> Option<Vec<Value>> {
    let completed_ids = completed_action_ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    if actions
        .iter()
        .filter(|action| action.status == "completed")
        .any(|action| !completed_ids.contains(&action.id))
        || completed_ids.iter().any(|id| {
            !actions
                .iter()
                .any(|action| action.id == *id && action.status == "completed")
        })
    {
        return None;
    }
    actions
        .iter()
        .filter(|action| action.status != "completed")
        .map(replay_safe_action_evidence)
        .collect()
}

fn interrupted_approved_executions(
    engine: &PersistenceEngine,
) -> rusqlite::Result<Vec<InterruptedApprovedExecution>> {
    let connection = engine.open_connection()?;
    let mut statement = connection.prepare(
        "SELECT executions.execution_id,executions.plan_id,executions.session_id,
                executions.agent_id,executions.provider_id,executions.model_id,
                executions.turn_id,executions.generation_token,executions.parent_turn_id,
                executions.root_turn_id,executions.turn_kind,executions.context_json,
                state.plan_json,state.current_step_index
         FROM agent_executions executions
         JOIN plan_generation_states state ON state.plan_id=executions.plan_id
         WHERE executions.status='running'
         ORDER BY executions.created_at_ms ASC,executions.execution_id ASC",
    )?;
    let rows = statement.query_map([], |row| {
        let next_step_index = row.get::<_, i64>(13)?;
        if next_step_index < 0 {
            return Err(rusqlite::Error::IntegralValueOutOfRange(
                13,
                next_step_index,
            ));
        }
        Ok(InterruptedApprovedExecution {
            execution_id: row.get(0)?,
            plan_id: row.get(1)?,
            context: ChatTurnPersistenceContext {
                session_id: row.get(2)?,
                agent_id: row.get(3)?,
                provider_id: row.get(4)?,
                model_id: row.get(5)?,
                turn_id: row.get(6)?,
                generation_token: row.get(7)?,
                parent_turn_id: row.get(8)?,
                root_turn_id: row.get(9)?,
                turn_kind: row.get(10)?,
            },
            context_json: row.get(11)?,
            plan_json: row.get(12)?,
            next_step_index: next_step_index as usize,
        })
    })?;
    rows.collect()
}

fn interrupted_approved_execution_recovery(
    engine: &PersistenceEngine,
    interrupted: InterruptedApprovedExecution,
) -> Option<InterruptedApprovedExecutionRecovery> {
    let request = serde_json::from_str::<crate::agentic_loop::AgentPlanExecutionRequest>(
        &interrupted.context_json,
    )
    .ok()?;
    if !request.principal_approved
        || request.authority_proof_id.is_some()
        || request.plan.id != interrupted.plan_id
        || request.turn_context.session_id != interrupted.context.session_id
        || request.turn_context.agent_id != interrupted.context.agent_id
        || request.turn_context.provider_id != interrupted.context.provider_id
        || request.turn_context.model_id != interrupted.context.model_id
        || request.turn_context.turn_id != interrupted.context.turn_id
        || request.turn_context.generation_token != interrupted.context.generation_token
        || request.turn_context.parent_turn_id != interrupted.context.parent_turn_id
        || request.turn_context.root_turn_id != interrupted.context.root_turn_id
        || request.turn_context.turn_kind != interrupted.context.turn_kind
        || serde_json::to_string(&request.plan).ok().as_deref()
            != Some(interrupted.plan_json.as_str())
        || interrupted.next_step_index == 0
        || interrupted.next_step_index >= request.plan.steps.len()
    {
        return None;
    }

    let checkpoint = engine
        .load_plan_execution_checkpoint(
            &interrupted.plan_id,
            &interrupted.plan_json,
            request.plan.steps.len(),
        )
        .ok()??;
    if checkpoint.next_step_index != interrupted.next_step_index {
        return None;
    }
    let connection = engine.open_connection().ok()?;
    let actions = durable_restart_actions(&connection, &interrupted.plan_id).ok()?;
    let completed_action_ids = checkpoint
        .completed_actions
        .iter()
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    let replay_safe_action_evidence =
        replay_safe_action_evidence_for_checkpoint(&actions, &completed_action_ids)?;
    let completed_outputs = checkpoint
        .completed_actions
        .iter()
        .map(|(_, output)| {
            serde_json::from_str::<crate::shield_gate::ExecuteCommandResponse>(output)
        })
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if completed_outputs
        .iter()
        .any(|output| !output.verified || output.status.as_str() != "completed")
    {
        return None;
    }

    let crate::agentic_loop::Tool::RegisteredTaskTool(planned) =
        &request.plan.steps[interrupted.next_step_index].tool
    else {
        return None;
    };
    if !crate::tools::task_tool_runtime::requires_explicit_approval(&planned.operation) {
        return None;
    }
    let planned_action = crate::tools::task_tool_runtime::requested_action(planned);
    let validated = crate::tools::task_tool_runtime::authorize(planned_action).ok()?;
    let resolved = crate::tools::task_tool_runtime::resolve(
        engine,
        Some(&interrupted.execution_id),
        validated,
        &completed_outputs,
    )
    .ok()?;
    let requested_action =
        crate::tools::task_tool_runtime::requested_action_for_validated(&resolved);
    let mut approval = crate::shield_gate::build_shield_approval_request(&requested_action)?;
    approval.session_id = Some(interrupted.context.session_id.clone());
    approval.turn_id = Some(interrupted.context.turn_id.clone());
    approval.generation_token = Some(interrupted.context.generation_token.clone());
    approval.principal = Some(interrupted.context.agent_id.clone());
    let frozen = crate::authority::shield_decision::freeze_request(&approval).ok()?;
    let completed_receipt_sha256s = checkpoint
        .completed_actions
        .iter()
        .map(|(_, output)| crate::foundation::digest::sha256_hex(output.as_bytes()))
        .collect::<Vec<_>>();
    let message = "OOMU restarted before the next confirmation. Your completed work is saved, and continuing will resume at the exact pending step.".to_string();
    let receipt_json = serde_json::json!({
        "schema": crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
        "executionId": interrupted.execution_id,
        "planId": interrupted.plan_id,
        "code": "agent_execution_interrupted",
        "boundary": "AgentExecutionRecovery",
        "recoverable": true,
        "recoveryAction": "resume_same_execution",
        "message": message,
        "context": {
            "completedStepCount": interrupted.next_step_index,
            "nextStepIndex": interrupted.next_step_index,
            "nextOperation": resolved.operation,
            "frozenArgumentSha256": frozen.argument_sha256,
            "completedReceiptSha256s": completed_receipt_sha256s,
            "replaySafeActionEvidence": replay_safe_action_evidence,
            "approvalTokenRetained": false,
            "approvalRequiredOnResume": true,
            "replayPolicy": "resume_from_verified_checkpoint"
        },
        "changedState": "checkpoint_saved"
    })
    .to_string();
    Some(InterruptedApprovedExecutionRecovery {
        execution_id: interrupted.execution_id,
        plan_id: interrupted.plan_id,
        context: interrupted.context,
        context_json: interrupted.context_json,
        message,
        receipt_json,
    })
}

impl PersistenceEngine {
    pub(super) fn persist_interrupted_approved_execution_recoveries(
        &self,
    ) -> rusqlite::Result<usize> {
        let interrupted = interrupted_approved_executions(self)?;
        let recoveries = interrupted
            .into_iter()
            .filter_map(|execution| interrupted_approved_execution_recovery(self, execution))
            .collect::<Vec<_>>();
        let mut persisted = 0;
        for recovery in recoveries {
            match self.finalize_agent_execution(
                &recovery.execution_id,
                &recovery.plan_id,
                &recovery.context,
                &recovery.context_json,
                "halted",
                Some(&recovery.receipt_json),
                "warn",
                "restart_recovery_ready",
                &recovery.message,
                Some(&recovery.receipt_json),
            ) {
                Ok(()) => persisted += 1,
                Err(error) => eprintln!(
                    "OOMU_AGENT_RESTART_TYPED_RECOVERY_SKIPPED execution_id={} error={}",
                    crate::redaction::redacted_log_text(&recovery.execution_id),
                    crate::redaction::redacted_log_text(&error.to_string())
                ),
            }
        }
        Ok(persisted)
    }
}

pub(super) fn restart_receipt_matches_pending_action(
    engine: &PersistenceEngine,
    execution_id: &str,
    plan_id: &str,
    context_json: &str,
    receipt_json: &str,
) -> bool {
    let Ok(connection) = engine.open_connection() else {
        return false;
    };
    restart_receipt_matches_pending_action_in_connection(
        engine,
        &connection,
        execution_id,
        plan_id,
        context_json,
        receipt_json,
    )
}

pub(super) fn restart_receipt_matches_pending_action_in_connection(
    engine: &PersistenceEngine,
    connection: &Connection,
    execution_id: &str,
    plan_id: &str,
    context_json: &str,
    receipt_json: &str,
) -> bool {
    let request = match serde_json::from_str::<crate::agentic_loop::AgentPlanExecutionRequest>(
        context_json,
    ) {
        Ok(request) => request,
        Err(_) => return false,
    };
    let state = connection.query_row(
        "SELECT plan_json,current_step_index FROM plan_generation_states WHERE plan_id=?1",
        params![plan_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    );
    let (plan_json, next_step_index) = match state {
        Ok((plan_json, next_step_index)) if next_step_index >= 0 => {
            (plan_json, next_step_index as usize)
        }
        _ => return false,
    };
    if !request.principal_approved
        || request.authority_proof_id.is_some()
        || request.plan.id != plan_id
        || serde_json::to_string(&request.plan).ok().as_deref() != Some(plan_json.as_str())
        || next_step_index == 0
        || next_step_index >= request.plan.steps.len()
    {
        return false;
    }
    let actions = match durable_restart_actions(connection, plan_id) {
        Ok(actions) => actions,
        Err(_) => return false,
    };
    let completed_actions = actions
        .iter()
        .filter(|action| action.status == "completed")
        .collect::<Vec<_>>();
    if completed_actions.len() != next_step_index {
        return false;
    }
    let completed_action_ids = completed_actions
        .iter()
        .map(|action| action.id)
        .collect::<Vec<_>>();
    let replay_safe_action_evidence =
        match replay_safe_action_evidence_for_checkpoint(&actions, &completed_action_ids) {
            Some(evidence) => evidence,
            None => return false,
        };
    let output_json = completed_actions
        .iter()
        .filter_map(|action| action.output.as_deref())
        .collect::<Vec<_>>();
    if output_json.len() != next_step_index {
        return false;
    }
    let outputs = match output_json
        .iter()
        .map(|output| serde_json::from_str::<crate::shield_gate::ExecuteCommandResponse>(output))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(outputs)
            if outputs
                .iter()
                .all(|output| output.verified && output.status.as_str() == "completed") =>
        {
            outputs
        }
        _ => return false,
    };
    let crate::agentic_loop::Tool::RegisteredTaskTool(planned) =
        &request.plan.steps[next_step_index].tool
    else {
        return false;
    };
    if !crate::tools::task_tool_runtime::requires_explicit_approval(&planned.operation) {
        return false;
    }
    let validated = match crate::tools::task_tool_runtime::authorize(
        crate::tools::task_tool_runtime::requested_action(planned),
    ) {
        Ok(validated) => validated,
        Err(_) => return false,
    };
    let resolved = match crate::tools::task_tool_runtime::resolve(
        engine,
        Some(execution_id),
        validated,
        &outputs,
    ) {
        Ok(resolved) => resolved,
        Err(_) => return false,
    };
    let action = crate::tools::task_tool_runtime::requested_action_for_validated(&resolved);
    let Some(mut approval) = crate::shield_gate::build_shield_approval_request(&action) else {
        return false;
    };
    approval.session_id = Some(request.turn_context.session_id.clone());
    approval.turn_id = Some(request.turn_context.turn_id.clone());
    approval.generation_token = Some(request.turn_context.generation_token.clone());
    approval.principal = Some(request.turn_context.agent_id.clone());
    let frozen = match crate::authority::shield_decision::freeze_request(&approval) {
        Ok(frozen) => frozen,
        Err(_) => return false,
    };
    let completed_receipt_sha256s = output_json
        .iter()
        .map(|output| crate::foundation::digest::sha256_hex(output.as_bytes()))
        .collect::<Vec<_>>();
    let Ok(receipt) = serde_json::from_str::<Value>(receipt_json) else {
        return false;
    };
    receipt.get("schema").and_then(Value::as_str)
        == Some(crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA)
        && receipt.get("executionId").and_then(Value::as_str) == Some(execution_id)
        && receipt.get("planId").and_then(Value::as_str) == Some(plan_id)
        && receipt.get("code").and_then(Value::as_str) == Some("agent_execution_interrupted")
        && receipt.get("boundary").and_then(Value::as_str) == Some("AgentExecutionRecovery")
        && receipt.get("recoverable").and_then(Value::as_bool) == Some(true)
        && receipt.get("recoveryAction").and_then(Value::as_str) == Some("resume_same_execution")
        && receipt.get("changedState").and_then(Value::as_str) == Some("checkpoint_saved")
        && receipt.pointer("/context/completedStepCount") == Some(&Value::from(next_step_index))
        && receipt.pointer("/context/nextStepIndex") == Some(&Value::from(next_step_index))
        && receipt
            .pointer("/context/nextOperation")
            .and_then(Value::as_str)
            == Some(resolved.operation)
        && receipt
            .pointer("/context/frozenArgumentSha256")
            .and_then(Value::as_str)
            == Some(frozen.argument_sha256.as_str())
        && receipt.pointer("/context/completedReceiptSha256s")
            == Some(&Value::from(completed_receipt_sha256s))
        && receipt.pointer("/context/replaySafeActionEvidence")
            == Some(&Value::from(replay_safe_action_evidence))
        && receipt.pointer("/context/approvalTokenRetained") == Some(&Value::Bool(false))
        && receipt.pointer("/context/approvalRequiredOnResume") == Some(&Value::Bool(true))
        && receipt
            .pointer("/context/replayPolicy")
            .and_then(Value::as_str)
            == Some("resume_from_verified_checkpoint")
}
