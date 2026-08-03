use super::{
    repository, ResolveTaskEffectVerificationRequest, TaskEffectVerificationDecision, TaskRunRecord,
};
use crate::{
    db::PersistenceEngine,
    p0_contracts::{EvidenceClass, P0EventEnvelope, TaskId, TaskRunId, TaskState},
};
use rusqlite::{params, OptionalExtension};
use serde_json::json;

#[derive(Clone, Debug)]
struct ExactEffectVerification {
    verification_sequence: u64,
    node_id: String,
    idempotency_key: String,
    effect_kind: String,
}

pub(super) fn resolve(
    engine: &PersistenceEngine,
    request: ResolveTaskEffectVerificationRequest,
) -> Result<TaskRunRecord, String> {
    engine.require_durable_store("resolve protected action verification")?;
    let task_run_id = TaskRunId::parse(&request.task_run_id)?.to_string();
    let task_id = TaskId::parse(&request.task_id)?.to_string();
    validate_runtime_identity(&request.runtime_record_id)?;

    let task = repository::get(engine, &task_run_id)?;
    if task.task_id != task_id
        || task.runtime_kind != "workflow"
        || task.runtime_record_id != request.runtime_record_id
        || task.state != TaskState::Blocked
        || task.recovery_state != "recoverable"
        || !task.effect_verification_required
    {
        return Err("The protected action decision no longer matches this Task.".to_string());
    }

    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    require_current_task_binding(
        &transaction,
        &task_run_id,
        &task_id,
        &request.runtime_record_id,
    )?;
    let exact = exact_verification_for_request(&transaction, &task_run_id, &task_id, &request)?;
    let verification_sequence = i64::try_from(exact.verification_sequence)
        .map_err(|_| "The protected action decision is stale.".to_string())?;
    require_exact_unresolved_event(
        &transaction,
        &task_run_id,
        &task_id,
        verification_sequence,
        &exact,
    )?;
    require_exact_effect_and_checkpoint(
        &transaction,
        &task_run_id,
        &request.runtime_record_id,
        &exact,
    )?;

    let now = crate::foundation::clock::unix_time_ms_i64();
    let (resolved_state, next_action) = match request.decision {
        TaskEffectVerificationDecision::DidNotHappen => {
            release_exact_effect_once(
                &transaction,
                &task_run_id,
                &request.runtime_record_id,
                &exact,
                now,
            )?;
            ("retry_requested", "retry_exact_effect_once")
        }
        TaskEffectVerificationDecision::Happened => {
            stop_after_observed_occurrence(
                &transaction,
                &task_run_id,
                &request.runtime_record_id,
                now,
            )?;
            ("stopped_unverified", "none")
        }
        TaskEffectVerificationDecision::StopWithoutRepeating => {
            stop_with_unknown_outcome(&transaction, &task_run_id, &request.runtime_record_id, now)?;
            ("stopped_outcome_unknown", "none")
        }
    };

    let decision = serde_json::to_value(request.decision)
        .map_err(|error| error.to_string())?
        .as_str()
        .ok_or_else(|| "The protected action decision is invalid.".to_string())?
        .to_string();
    transaction
        .execute(
            "INSERT INTO task_recovery_audit(task_run_id,previous_state,resolved_state,decision,next_action,created_at_ms) VALUES (?1,'effect_verification_required',?2,?3,?4,?5)",
            params![task_run_id, resolved_state, decision, next_action, now],
        )
        .map_err(|error| error.to_string())?;
    repository::append_event_in_transaction(
        &transaction,
        &task,
        "workflow.effect.verification_resolved",
        EvidenceClass::ExecutedMutation,
        json!({
            "verificationSequence": exact.verification_sequence,
            "nodeId": exact.node_id,
            "idempotencyKey": exact.idempotency_key,
            "effectKind": exact.effect_kind,
            "decision": decision,
            "nextAction": next_action,
            "runtimeRecordId": request.runtime_record_id,
            "outcome": match request.decision {
                TaskEffectVerificationDecision::DidNotHappen => "observed_not_happened",
                TaskEffectVerificationDecision::Happened => "observed_happened",
                TaskEffectVerificationDecision::StopWithoutRepeating => "outcome_unknown",
            },
            "postconditionVerified": false,
        }),
    )?
    .ok_or_else(|| "Protected action recovery requires a Project audit binding.".to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    repository::get(engine, &task_run_id)
}

fn validate_runtime_identity(runtime_record_id: &str) -> Result<(), String> {
    validate_identity("runtime", runtime_record_id, 256)
}

fn exact_verification_for_request(
    transaction: &rusqlite::Transaction<'_>,
    task_run_id: &str,
    task_id: &str,
    request: &ResolveTaskEffectVerificationRequest,
) -> Result<ExactEffectVerification, String> {
    let exact = match (
        request.verification_sequence,
        request.node_id.as_deref(),
        request.idempotency_key.as_deref(),
        request.effect_kind.as_deref(),
    ) {
        (Some(verification_sequence), Some(node_id), Some(idempotency_key), Some(effect_kind)) => {
            ExactEffectVerification {
                verification_sequence,
                node_id: node_id.to_string(),
                idempotency_key: idempotency_key.to_string(),
                effect_kind: effect_kind.to_string(),
            }
        }
        (None, None, None, None)
            if matches!(
                request.decision,
                TaskEffectVerificationDecision::StopWithoutRepeating
            ) =>
        {
            derive_single_unresolved_verification(transaction, task_run_id, task_id)?
        }
        (None, None, None, None) => {
            return Err("The protected action details are required for this decision.".to_string())
        }
        _ => {
            return Err(
                "The protected action details must be supplied together or omitted together."
                    .to_string(),
            )
        }
    };
    validate_exact_identity(&exact)?;
    Ok(exact)
}

fn derive_single_unresolved_verification(
    transaction: &rusqlite::Transaction<'_>,
    task_run_id: &str,
    task_id: &str,
) -> Result<ExactEffectVerification, String> {
    let mut statement = transaction
        .prepare(
            "SELECT required.sequence,required.event_json FROM task_events required \
             WHERE required.task_run_id=?1 \
             AND json_extract(required.event_json,'$.eventType')='workflow.effect.verification_required' \
             AND NOT EXISTS(SELECT 1 FROM task_events resolved \
                 WHERE resolved.task_run_id=required.task_run_id \
                 AND resolved.sequence>required.sequence \
                 AND json_extract(resolved.event_json,'$.eventType')='workflow.effect.verification_resolved' \
                 AND json_extract(resolved.event_json,'$.payload.verificationSequence')=required.sequence) \
             ORDER BY required.sequence LIMIT 2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![task_run_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let [(sequence, raw_event)] = rows.as_slice() else {
        return Err(if rows.is_empty() {
            "The protected action decision is stale.".to_string()
        } else {
            "More than one protected action still needs verification. Load the exact action details before stopping this run."
                .to_string()
        });
    };
    let sequence = u64::try_from(*sequence)
        .map_err(|_| "The protected action decision is stale.".to_string())?;
    let event: P0EventEnvelope = serde_json::from_str(raw_event)
        .map_err(|_| "The protected action audit is invalid.".to_string())?;
    if event.event_type != "workflow.effect.verification_required"
        || event.sequence != sequence
        || event.task_id.as_str() != task_id
        || event.task_run_id.as_ref().map(TaskRunId::as_str) != Some(task_run_id)
        || event
            .payload
            .get("nextAction")
            .and_then(serde_json::Value::as_str)
            != Some("verify_only")
    {
        return Err(
            "The protected action decision no longer matches its saved boundary.".to_string(),
        );
    }
    let exact = ExactEffectVerification {
        verification_sequence: sequence,
        node_id: required_event_identity(&event, "nodeId")?,
        idempotency_key: required_event_identity(&event, "idempotencyKey")?,
        effect_kind: required_event_identity(&event, "effectKind")?,
    };
    validate_exact_identity(&exact)?;
    Ok(exact)
}

fn required_event_identity(event: &P0EventEnvelope, key: &str) -> Result<String, String> {
    event
        .payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "The protected action audit is invalid.".to_string())
}

fn validate_exact_identity(exact: &ExactEffectVerification) -> Result<(), String> {
    for (label, value, maximum) in [
        ("node", exact.node_id.as_str(), 256),
        ("effect", exact.idempotency_key.as_str(), 1_024),
        ("effect kind", exact.effect_kind.as_str(), 256),
    ] {
        validate_identity(label, value, maximum)?;
    }
    Ok(())
}

fn validate_identity(label: &str, value: &str, maximum: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(format!("The protected action {label} identity is invalid."));
    }
    Ok(())
}

fn require_current_task_binding(
    transaction: &rusqlite::Transaction<'_>,
    task_run_id: &str,
    task_id: &str,
    runtime_record_id: &str,
) -> Result<(), String> {
    let current: (String, String, String, String, String) = transaction
        .query_row(
            "SELECT task_id,runtime_kind,runtime_record_id,state,recovery_state FROM task_runs WHERE task_run_id=?1",
            params![task_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|error| error.to_string())?;
    if current
        != (
            task_id.to_string(),
            "workflow".to_string(),
            runtime_record_id.to_string(),
            "blocked".to_string(),
            "recoverable".to_string(),
        )
    {
        return Err("The protected action decision no longer matches this Task.".to_string());
    }
    Ok(())
}

fn require_exact_unresolved_event(
    transaction: &rusqlite::Transaction<'_>,
    task_run_id: &str,
    task_id: &str,
    verification_sequence: i64,
    exact: &ExactEffectVerification,
) -> Result<(), String> {
    let raw_event: String = transaction
        .query_row(
            "SELECT event_json FROM task_events WHERE task_run_id=?1 AND sequence=?2",
            params![task_run_id, verification_sequence],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The protected action decision is stale.".to_string())?;
    let event: P0EventEnvelope = serde_json::from_str(&raw_event)
        .map_err(|_| "The protected action audit is invalid.".to_string())?;
    if event.event_type != "workflow.effect.verification_required"
        || event.sequence != exact.verification_sequence
        || event.task_id.as_str() != task_id
        || event.task_run_id.as_ref().map(TaskRunId::as_str) != Some(task_run_id)
        || event
            .payload
            .get("nodeId")
            .and_then(serde_json::Value::as_str)
            != Some(exact.node_id.as_str())
        || event
            .payload
            .get("idempotencyKey")
            .and_then(serde_json::Value::as_str)
            != Some(exact.idempotency_key.as_str())
        || event
            .payload
            .get("effectKind")
            .and_then(serde_json::Value::as_str)
            != Some(exact.effect_kind.as_str())
        || event
            .payload
            .get("nextAction")
            .and_then(serde_json::Value::as_str)
            != Some("verify_only")
    {
        return Err(
            "The protected action decision no longer matches its saved boundary.".to_string(),
        );
    }

    let latest_unresolved: Option<i64> = transaction
        .query_row(
            "SELECT MAX(required.sequence) FROM task_events required WHERE required.task_run_id=?1 AND json_extract(required.event_json,'$.eventType')='workflow.effect.verification_required' AND json_extract(required.event_json,'$.payload.nodeId')=?2 AND json_extract(required.event_json,'$.payload.idempotencyKey')=?3 AND json_extract(required.event_json,'$.payload.effectKind')=?4 AND NOT EXISTS(SELECT 1 FROM task_events resolved WHERE resolved.task_run_id=required.task_run_id AND resolved.sequence>required.sequence AND json_extract(resolved.event_json,'$.eventType')='workflow.effect.verification_resolved' AND json_extract(resolved.event_json,'$.payload.nodeId')=?2 AND json_extract(resolved.event_json,'$.payload.idempotencyKey')=?3 AND json_extract(resolved.event_json,'$.payload.effectKind')=?4)",
            params![task_run_id, exact.node_id, exact.idempotency_key, exact.effect_kind],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if latest_unresolved != Some(verification_sequence) {
        return Err("The protected action decision is stale.".to_string());
    }
    Ok(())
}

fn require_exact_effect_and_checkpoint(
    transaction: &rusqlite::Transaction<'_>,
    task_run_id: &str,
    runtime_record_id: &str,
    exact: &ExactEffectVerification,
) -> Result<(), String> {
    let effect: Option<(String, String, Option<String>)> = transaction
        .query_row(
            "SELECT effect_kind,state,result_digest FROM task_effects WHERE task_run_id=?1 AND idempotency_key=?2",
            params![task_run_id, exact.idempotency_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if effect != Some((exact.effect_kind.clone(), "executed".to_string(), None)) {
        return Err(
            "The protected action has changed since OOMU asked you to inspect it.".to_string(),
        );
    }
    let (instance_status, node_payloads): (String, String) = transaction
        .query_row(
            "SELECT status,node_payloads_json FROM execution_instances WHERE id=?1",
            params![runtime_record_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let node_payloads: serde_json::Value = serde_json::from_str(&node_payloads)
        .map_err(|_| "The stopped workflow checkpoint is invalid.".to_string())?;
    if instance_status != "Failed"
        || node_payloads
            .get(&exact.node_id)
            .and_then(|payload| payload.pointer("/error/code"))
            .and_then(serde_json::Value::as_str)
            != Some("workflow_effect_verification_required")
    {
        return Err("The workflow is no longer stopped at this protected action.".to_string());
    }
    Ok(())
}

fn release_exact_effect_once(
    transaction: &rusqlite::Transaction<'_>,
    task_run_id: &str,
    runtime_record_id: &str,
    exact: &ExactEffectVerification,
    now: i64,
) -> Result<(), String> {
    let schedule_id: String = transaction
        .query_row(
            "SELECT schedule_id FROM routine_runs WHERE execution_instance_id=?1 AND task_run_id=?2",
            params![runtime_record_id, task_run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            "This protected action is not bound to a recoverable scheduled run.".to_string()
        })?;
    let released = transaction
        .execute(
            "DELETE FROM task_effects WHERE task_run_id=?1 AND idempotency_key=?2 AND effect_kind=?3 AND state='executed' AND result_digest IS NULL",
            params![task_run_id, exact.idempotency_key, exact.effect_kind],
        )
        .map_err(|error| error.to_string())?;
    let instance = transaction
        .execute(
            "UPDATE execution_instances SET status='Pending',active_node_id=NULL,error_json=NULL,completed_at_ms=NULL,updated_at_ms=?2 WHERE id=?1 AND status='Failed'",
            params![runtime_record_id, now],
        )
        .map_err(|error| error.to_string())?;
    let schedule = transaction
        .execute(
            "UPDATE workflow_schedules SET is_active=1,claimed_at_ms=NULL,next_run_at_ms=?2,last_status='Pending',last_error=NULL,last_instance_id=?3,updated_at_ms=?2 WHERE id=?1 AND last_instance_id=?3",
            params![schedule_id, now, runtime_record_id],
        )
        .map_err(|error| error.to_string())?;
    let task_change = transaction
        .execute(
            "UPDATE task_runs SET state='queued',last_error=NULL,recovery_state='reconciled',completed_at_ms=NULL,updated_at_ms=?2 WHERE task_run_id=?1 AND state='blocked' AND recovery_state='recoverable'",
            params![task_run_id, now],
        )
        .map_err(|error| error.to_string())?;
    if released != 1 || instance != 1 || schedule != 1 || task_change != 1 {
        return Err("OOMU could not preserve the exact retry boundary.".to_string());
    }
    Ok(())
}

fn stop_after_observed_occurrence(
    transaction: &rusqlite::Transaction<'_>,
    task_run_id: &str,
    runtime_record_id: &str,
    now: i64,
) -> Result<(), String> {
    let stop_error = serde_json::to_string(&json!({
        "code": "workflow_effect_stopped_after_user_observation",
        "message": "The user observed that the protected action happened. OOMU stopped without repeating it and did not mark the workflow complete."
    }))
    .map_err(|error| error.to_string())?;
    let instance = transaction
        .execute(
            "UPDATE execution_instances SET status='Failed',active_node_id=NULL,error_json=?2,completed_at_ms=COALESCE(completed_at_ms,?3),updated_at_ms=?3 WHERE id=?1 AND status='Failed'",
            params![runtime_record_id, stop_error, now],
        )
        .map_err(|error| error.to_string())?;
    let task_change = transaction
        .execute(
            "UPDATE task_runs SET state='failed',last_error='The protected action happened, so OOMU stopped without repeating it. This Task was not marked complete.',recovery_state='reconciled',completed_at_ms=?2,updated_at_ms=?2 WHERE task_run_id=?1 AND state='blocked' AND recovery_state='recoverable'",
            params![task_run_id, now],
        )
        .map_err(|error| error.to_string())?;
    if instance != 1 || task_change != 1 {
        return Err("OOMU could not preserve the stop-without-repeating decision.".to_string());
    }
    Ok(())
}

fn stop_with_unknown_outcome(
    transaction: &rusqlite::Transaction<'_>,
    task_run_id: &str,
    runtime_record_id: &str,
    now: i64,
) -> Result<(), String> {
    let stop_error = serde_json::to_string(&json!({
        "code": "workflow_effect_stopped_outcome_unknown",
        "message": "The user stopped this run without repeating the protected action. Its outcome remains unknown, and OOMU did not mark the workflow complete."
    }))
    .map_err(|error| error.to_string())?;
    let instance = transaction
        .execute(
            "UPDATE execution_instances SET status='Failed',active_node_id=NULL,error_json=?2,completed_at_ms=COALESCE(completed_at_ms,?3),updated_at_ms=?3 WHERE id=?1 AND status='Failed'",
            params![runtime_record_id, stop_error, now],
        )
        .map_err(|error| error.to_string())?;
    let task_change = transaction
        .execute(
            "UPDATE task_runs SET state='cancelled',last_error='The protected action outcome is unknown. OOMU stopped this run without repeating it.',recovery_state='reconciled',completed_at_ms=?2,updated_at_ms=?2 WHERE task_run_id=?1 AND state='blocked' AND recovery_state='recoverable'",
            params![task_run_id, now],
        )
        .map_err(|error| error.to_string())?;
    if instance != 1 || task_change != 1 {
        return Err("OOMU could not preserve the stop-without-repeating decision.".to_string());
    }
    Ok(())
}
