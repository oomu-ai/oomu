//! Durable effect reservation, replay, verification, and recovery reconciliation.
use super::WorkflowRuntimeError;
use crate::{
    db::PersistenceEngine,
    p0_contracts::EvidenceClass,
    tools::{task_runtime, task_runtime::require_agent_runtime_task},
};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

#[derive(Clone, Debug)]
pub(super) struct ReservedEffect {
    pub(super) task_run_id: String,
    pub(super) node_id: String,
    pub(super) key: String,
    pub(super) operation: String,
    pub(super) summary: Value,
}

#[derive(Debug)]
pub(super) enum EffectReservation {
    Execute(ReservedEffect),
    Replay(Value),
}

struct VerifiedEffectReceipt {
    digest: String,
    result: Value,
}

pub(super) fn reserve_effect(
    persistence: &PersistenceEngine,
    execution_id: &str,
    node_id: &str,
    operation: &str,
    arguments: &Value,
) -> Result<EffectReservation, WorkflowRuntimeError> {
    let task = require_agent_runtime_task(persistence, execution_id)
        .map_err(|message| WorkflowRuntimeError::new("workflow_task_binding_missing", message))?;
    reserve_effect_for_task(
        persistence,
        &task.task_run_id,
        node_id,
        operation,
        arguments,
    )
}

pub(super) fn reserve_effect_for_task(
    persistence: &PersistenceEngine,
    task_run_id: &str,
    node_id: &str,
    operation: &str,
    arguments: &Value,
) -> Result<EffectReservation, WorkflowRuntimeError> {
    let summary = effect_recovery_summary(operation, arguments);
    let arguments = serde_json::to_vec(arguments).map_err(WorkflowRuntimeError::serialization)?;
    let key = format!(
        "workflow-task:{node_id}:{operation}:{}",
        crate::foundation::digest::sha256_hex(&arguments)
    );
    let effect = ReservedEffect {
        task_run_id: task_run_id.to_string(),
        node_id: node_id.to_string(),
        key,
        operation: operation.to_string(),
        summary,
    };
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?;
    let inserted = connection
        .execute(
            "INSERT INTO task_effects(task_run_id,idempotency_key,effect_kind,state,updated_at_ms) VALUES (?1,?2,?3,'reserved',?4) ON CONFLICT(task_run_id,idempotency_key) DO NOTHING",
            params![effect.task_run_id, effect.key, effect.operation, now],
        )
        .map_err(WorkflowRuntimeError::database)?;
    let bound: Option<(String, String, Option<String>)> = connection
        .query_row(
            "SELECT effect_kind,state,result_digest FROM task_effects WHERE task_run_id=?1 AND idempotency_key=?2",
            params![effect.task_run_id, effect.key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(WorkflowRuntimeError::database)?;
    let Some((bound_operation, state, stored_digest)) = bound else {
        return Err(WorkflowRuntimeError::new(
            "workflow_effect_reservation_lost",
            "The scheduled effect reservation could not be read back.".to_string(),
        ));
    };
    if bound_operation != operation {
        return Err(WorkflowRuntimeError::new(
            "workflow_effect_binding_changed",
            "The scheduled effect no longer matches its durable reservation.".to_string(),
        ));
    }
    match load_verified_effect_receipt(persistence, &effect) {
        Ok(Some(receipt)) => {
            if stored_digest
                .as_deref()
                .is_some_and(|digest| digest != receipt.digest)
            {
                return halt_for_effect_verification(
                    persistence,
                    &effect,
                    "workflow_effect_receipt_digest_changed",
                );
            }
            if let Err(error) = reconcile_verified_effect(persistence, &effect, &receipt.digest) {
                return halt_for_effect_verification(persistence, &effect, error.code);
            }
            return Ok(EffectReservation::Replay(receipt.result));
        }
        Ok(None) => {}
        Err(error) => return halt_for_effect_verification(persistence, &effect, error.code),
    }
    if inserted == 1 && state == "reserved" {
        return Ok(EffectReservation::Execute(effect));
    }
    halt_for_effect_verification(
        persistence,
        &effect,
        if state == "verified" {
            "workflow_effect_verified_receipt_missing"
        } else {
            "workflow_effect_execution_ambiguous"
        },
    )
}

pub(super) fn claim_effect_execution(
    persistence: &PersistenceEngine,
    effect: &ReservedEffect,
) -> Result<Option<Value>, WorkflowRuntimeError> {
    let changed = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?
        .execute(
            "UPDATE task_effects SET state='executed',updated_at_ms=?3 WHERE task_run_id=?1 AND idempotency_key=?2 AND effect_kind=?4 AND state='reserved'",
            params![
                effect.task_run_id,
                effect.key,
                crate::foundation::clock::unix_time_ms_i64(),
                effect.operation,
            ],
        )
        .map_err(WorkflowRuntimeError::database)?;
    if changed == 1 {
        return Ok(None);
    }
    match load_verified_effect_receipt(persistence, effect) {
        Ok(Some(receipt)) => {
            if let Err(error) = reconcile_verified_effect(persistence, effect, &receipt.digest) {
                return halt_for_effect_verification(persistence, effect, error.code);
            }
            Ok(Some(receipt.result))
        }
        Ok(None) => {
            halt_for_effect_verification(persistence, effect, "workflow_effect_execution_ambiguous")
        }
        Err(error) => halt_for_effect_verification(persistence, effect, error.code),
    }
}

pub(super) fn verify_effect(
    persistence: &PersistenceEngine,
    effect: &ReservedEffect,
    result: &Value,
) -> Result<(), WorkflowRuntimeError> {
    let encoded = serde_json::to_vec(result).map_err(WorkflowRuntimeError::serialization)?;
    let digest = crate::foundation::digest::sha256_hex(&encoded);
    if let Some(receipt) = load_verified_effect_receipt(persistence, effect)? {
        if receipt.digest != digest || receipt.result != *result {
            return Err(WorkflowRuntimeError::new(
                "workflow_effect_receipt_conflict",
                "The durable effect receipt does not match the verified result.".to_string(),
            ));
        }
        return reconcile_verified_effect(persistence, effect, &digest);
    }
    task_runtime::record_event(
        persistence,
        &effect.task_run_id,
        "workflow.effect.verified",
        EvidenceClass::VerifiedPostcondition,
        serde_json::json!({
            "idempotencyKey": effect.key,
            "effectKind": effect.operation,
            "resultDigest": digest,
            "result": result,
        }),
    )
    .map_err(|message| {
        WorkflowRuntimeError::new("workflow_effect_receipt_persistence_failed", message)
    })?;
    reconcile_verified_effect(persistence, effect, &digest)
}

fn reconcile_verified_effect(
    persistence: &PersistenceEngine,
    effect: &ReservedEffect,
    digest: &str,
) -> Result<(), WorkflowRuntimeError> {
    let changed = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?
        .execute(
            "UPDATE task_effects SET state='verified',result_digest=?3,updated_at_ms=?4 WHERE task_run_id=?1 AND idempotency_key=?2 AND effect_kind=?5 AND state IN ('reserved','executed','verified') AND (result_digest IS NULL OR result_digest=?3)",
            params![effect.task_run_id, effect.key, digest, crate::foundation::clock::unix_time_ms_i64(), effect.operation],
        )
        .map_err(WorkflowRuntimeError::database)?;
    if changed != 1 {
        return Err(WorkflowRuntimeError::new(
            "workflow_effect_verification_lost",
            "The scheduled effect completed, but its durable receipt could not be verified. OOMU stopped before reporting success.".to_string(),
        ));
    }
    Ok(())
}

fn load_verified_effect_receipt(
    persistence: &PersistenceEngine,
    effect: &ReservedEffect,
) -> Result<Option<VerifiedEffectReceipt>, WorkflowRuntimeError> {
    let raw = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?
        .query_row(
            "SELECT event_json FROM task_events WHERE task_run_id=?1 AND json_extract(event_json,'$.eventType')='workflow.effect.verified' AND json_extract(event_json,'$.payload.idempotencyKey')=?2 ORDER BY sequence DESC LIMIT 1",
            params![effect.task_run_id, effect.key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(WorkflowRuntimeError::database)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let event: Value = serde_json::from_str(&raw).map_err(WorkflowRuntimeError::serialization)?;
    let payload = event
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            WorkflowRuntimeError::new(
                "workflow_effect_receipt_invalid",
                "The durable effect receipt has no verified payload.".to_string(),
            )
        })?;
    if payload.get("idempotencyKey").and_then(Value::as_str) != Some(effect.key.as_str())
        || payload.get("effectKind").and_then(Value::as_str) != Some(effect.operation.as_str())
    {
        return Err(WorkflowRuntimeError::new(
            "workflow_effect_receipt_invalid",
            "The durable effect receipt no longer matches its reservation.".to_string(),
        ));
    }
    let digest = payload
        .get("resultDigest")
        .and_then(Value::as_str)
        .filter(|value| {
            value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
        })
        .ok_or_else(|| {
            WorkflowRuntimeError::new(
                "workflow_effect_receipt_invalid",
                "The durable effect receipt has no valid result digest.".to_string(),
            )
        })?
        .to_string();
    let result = payload.get("result").cloned().ok_or_else(|| {
        WorkflowRuntimeError::new(
            "workflow_effect_receipt_invalid",
            "The durable effect receipt has no replayable result.".to_string(),
        )
    })?;
    let actual = serde_json::to_vec(&result).map_err(WorkflowRuntimeError::serialization)?;
    if crate::foundation::digest::sha256_hex(&actual) != digest {
        return Err(WorkflowRuntimeError::new(
            "workflow_effect_receipt_digest_changed",
            "The durable effect receipt failed its integrity check.".to_string(),
        ));
    }
    Ok(Some(VerifiedEffectReceipt { digest, result }))
}

pub(super) fn release_unchanged_effect(
    persistence: &PersistenceEngine,
    effect: &ReservedEffect,
    error_code: &str,
) -> Result<(), WorkflowRuntimeError> {
    let changed = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?
        .execute(
            "DELETE FROM task_effects WHERE task_run_id=?1 AND idempotency_key=?2 AND effect_kind=?3 AND state='executed' AND result_digest IS NULL",
            params![effect.task_run_id, effect.key, effect.operation],
        )
        .map_err(WorkflowRuntimeError::database)?;
    if changed != 1 {
        return halt_for_effect_verification(
            persistence,
            effect,
            "workflow_effect_unchanged_release_failed",
        );
    }
    task_runtime::record_event(
        persistence,
        &effect.task_run_id,
        "workflow.effect.unchanged",
        EvidenceClass::ObservedResult,
        serde_json::json!({
            "idempotencyKey": effect.key,
            "effectKind": effect.operation,
            "errorCode": error_code,
            "changedState": false,
        }),
    )
    .map_err(|message| {
        WorkflowRuntimeError::new("workflow_effect_receipt_persistence_failed", message)
    })
}

pub(super) fn halt_for_effect_verification<T>(
    persistence: &PersistenceEngine,
    effect: &ReservedEffect,
    reason_code: &str,
) -> Result<T, WorkflowRuntimeError> {
    let message = "OOMU could not confirm whether this protected action already happened. It will not repeat the action automatically. The Task is saved for verification-only recovery."
        .to_string();
    let effect_changed = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?
        .execute(
            "UPDATE task_effects SET state='executed',updated_at_ms=?4 WHERE task_run_id=?1 AND idempotency_key=?2 AND effect_kind=?3 AND state IN ('reserved','executed','verified')",
            params![effect.task_run_id, effect.key, effect.operation, crate::foundation::clock::unix_time_ms_i64()],
        )
        .map_err(WorkflowRuntimeError::database)?;
    if effect_changed != 1 {
        return Err(WorkflowRuntimeError::new(
            "workflow_effect_recovery_persistence_failed",
            "OOMU stopped the external action, but could not preserve its verification boundary."
                .to_string(),
        ));
    }
    let changed = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?
        .execute(
            "UPDATE task_runs SET state=CASE WHEN state IN ('queued','planning','awaiting_approval','running') THEN 'blocked' ELSE state END,recovery_state='recoverable',last_error=?2,updated_at_ms=?3 WHERE task_run_id=?1",
            params![effect.task_run_id, message, crate::foundation::clock::unix_time_ms_i64()],
        )
        .map_err(WorkflowRuntimeError::database)?;
    if changed != 1 {
        return Err(WorkflowRuntimeError::new(
            "workflow_effect_recovery_persistence_failed",
            "OOMU stopped the external action, but could not save its recovery state.".to_string(),
        ));
    }
    let retry_supported = persistence
        .open_connection()
        .and_then(|connection| {
            connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM task_runs task JOIN routine_runs run ON run.execution_instance_id=task.runtime_record_id AND run.task_run_id=task.task_run_id WHERE task.task_run_id=?1 AND task.runtime_kind='workflow')",
                params![effect.task_run_id],
                |row| row.get::<_, bool>(0),
            )
        })
        .unwrap_or(false);
    task_runtime::record_event(
        persistence,
        &effect.task_run_id,
        "workflow.effect.verification_required",
        EvidenceClass::ObservedResult,
        serde_json::json!({
            "nodeId": effect.node_id,
            "idempotencyKey": effect.key,
            "effectKind": effect.operation,
            "effectSummary": effect.summary,
            "reasonCode": reason_code,
            "nextAction": "verify_only",
            "retrySupported": retry_supported,
        }),
    )
    .map_err(|detail| {
        WorkflowRuntimeError::new(
            "workflow_effect_recovery_persistence_failed",
            format!("{message} Recovery audit failed: {detail}"),
        )
    })?;
    Err(WorkflowRuntimeError::new(
        "workflow_effect_verification_required",
        message,
    ))
}

fn effect_recovery_summary(operation: &str, arguments: &Value) -> Value {
    let text = |field: &str, limit: usize| {
        arguments
            .get(field)
            .and_then(Value::as_str)
            .and_then(|value| bounded_recovery_text(value, limit))
    };
    match operation {
        "create_system_calendar_event" | "create_conflict_free_calendar_event" => {
            serde_json::json!({
                "surface": "calendar",
                "calendarName": text("calendarName", 160),
                "title": text("title", 240),
            })
        }
        "draft_system_email" | "draft_decision_pack_email" | "draft_release_recovery_email" => {
            serde_json::json!({
                "surface": "mail_draft",
                "recipient": text("to", 4_096),
                "subject": text("subject", 998),
            })
        }
        "send_system_email" => serde_json::json!({
            "surface": "mail_send",
            "recipient": text("to", 4_096),
            "subject": text("subject", 998),
        }),
        _ => serde_json::json!({ "surface": "protected_action" }),
    }
}

fn bounded_recovery_text(value: &str, limit: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.chars().count() <= limit
        && !value.chars().any(|character| character.is_control()))
    .then(|| value.to_string())
}
