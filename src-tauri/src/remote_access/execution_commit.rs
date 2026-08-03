use super::{
    artifact_transfer::{self, PreparedArtifactGrant},
    command_store::{self, StoredRemoteCommand},
    crypto, RemoteCommandResult, SignedRemoteCommand,
};
use crate::{
    db::PersistenceEngine,
    p0_contracts::{
        EvidenceClass, P0EventEnvelope, ProjectId, TaskId, TaskRunId, P0_CONTRACT_VERSION,
    },
    sovereign_identity::SovereignIdentity,
};
use chrono::{SecondsFormat, TimeZone, Utc};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

#[derive(Debug)]
struct FinalOutcome {
    status: &'static str,
    code: &'static str,
    message: &'static str,
}

fn outcome_for(command_kind: &str) -> FinalOutcome {
    if matches!(command_kind, "view_task" | "stop_task" | "request_artifact") {
        FinalOutcome {
            status: "completed",
            code: "applied",
            message: "Your Mac completed this request.",
        }
    } else {
        FinalOutcome {
            status: "rejected",
            code: "local_review_required",
            message: "Open OOMU on your Mac to finish this request safely.",
        }
    }
}

fn current_sequence(transaction: &Transaction<'_>, task_run_id: &str) -> Result<u64, String> {
    let value = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence),-1)+1 FROM task_events WHERE task_run_id=?1",
            params![task_run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|cause| format!("remote_execution_sequence_read_failed: {cause}"))?;
    u64::try_from(value)
        .map_err(|_| "remote_execution_sequence_invalid: The Task sequence is invalid.".to_string())
}

fn apply_stop_task(
    transaction: &Transaction<'_>,
    command: &StoredRemoteCommand,
    sequence: u64,
    now: i64,
) -> Result<(), String> {
    let task_run_id = command
        .task_run_id
        .as_deref()
        .ok_or_else(|| "remote_execution_task_required: Choose a Task to stop.".to_string())?;
    let task = transaction
        .query_row(
            "SELECT task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id FROM task_runs WHERE task_run_id=?1",
            params![task_run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|cause| format!("remote_execution_task_read_failed: {cause}"))?
        .ok_or_else(|| "remote_execution_task_missing: This Task no longer exists.".to_string())?;
    if task.1.as_deref() != Some(command.project_id.as_str()) {
        return Err(
            "remote_execution_task_scope_mismatch: This Task is outside the device’s Project access."
                .to_string(),
        );
    }
    if !matches!(
        task.4.as_str(),
        "queued" | "planning" | "awaiting_approval" | "running" | "blocked"
    ) {
        return Err(
            "remote_execution_task_not_stoppable: This Task has already finished.".to_string(),
        );
    }
    let runtime_changed = match task.2.as_str() {
        "taskflow" => {
            transaction
                .execute(
                    "UPDATE taskflow_steps SET status='cancelled' WHERE flow_id=?1 AND status IN ('queued','active')",
                    params![task.3],
                )
                .map_err(|cause| format!("remote_execution_task_stop_failed: {cause}"))?;
            transaction.execute(
                "UPDATE taskflows SET status='cancelled',updated_at_ms=?2 WHERE flow_id=?1 AND status NOT IN ('verified','cancelled')",
                params![task.3, now],
            )
        }
        "agent" => transaction.execute(
            "UPDATE agent_executions SET status='cancelled',updated_at_ms=?2 WHERE execution_id=?1 AND status IN ('running','halted')",
            params![task.3, now],
        ),
        "queued_message" => transaction.execute(
            "UPDATE message_queue SET status='cancelled',updated_at_ms=?2,error_message='Cancelled from a trusted remote device.' WHERE CAST(id AS TEXT)=?1 AND status='queued'",
            params![task.3, now],
        ),
        _ => {
            return Err(
                "remote_execution_task_not_stoppable: This Task cannot be stopped at its current safe boundary."
                    .to_string(),
            )
        }
    }
    .map_err(|cause| format!("remote_execution_task_stop_failed: {cause}"))?;
    if runtime_changed != 1 {
        return Err(
            "remote_execution_task_stop_rejected: The Task changed before it could be stopped."
                .to_string(),
        );
    }
    let task_changed = transaction
        .execute(
            "UPDATE task_runs SET state='cancelled',updated_at_ms=?2,completed_at_ms=?2,recovery_state='not_required' WHERE task_run_id=?1 AND state IN ('queued','planning','awaiting_approval','running','blocked')",
            params![task_run_id, now],
        )
        .map_err(|cause| format!("remote_execution_task_finalize_failed: {cause}"))?;
    if task_changed != 1 {
        return Err(
            "remote_execution_task_finalize_failed: The Task could not be finalized.".to_string(),
        );
    }
    let timestamp = Utc
        .timestamp_millis_opt(now)
        .single()
        .ok_or_else(|| "remote_execution_time_invalid: The Task time is invalid.".to_string())?
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let event = P0EventEnvelope {
        schema_version: P0_CONTRACT_VERSION,
        event_type: "task.cancel".to_string(),
        project_id: ProjectId::parse(&command.project_id)?,
        task_id: TaskId::parse(&task.0)?,
        task_run_id: Some(TaskRunId::parse(task_run_id)?),
        correlation_id: task.6,
        sequence,
        timestamp,
        evidence_class: EvidenceClass::ExecutedMutation,
        payload: serde_json::json!({
            "previousState": task.4,
            "state": "cancelled",
            "runtime": task.2,
            "origin": task.5,
            "source": "trusted_remote_device",
        }),
    };
    transaction
        .execute(
            "INSERT INTO task_events (task_run_id,sequence,event_json,created_at_ms) VALUES (?1,?2,?3,?4)",
            params![
                task_run_id,
                i64::try_from(sequence).map_err(|_| {
                    "remote_execution_sequence_invalid: The Task sequence is invalid.".to_string()
                })?,
                serde_json::to_string(&event)
                    .map_err(|cause| format!("remote_execution_event_invalid: {cause}"))?,
                now,
            ],
        )
        .map_err(|cause| format!("remote_execution_event_store_failed: {cause}"))?;
    Ok(())
}

fn signed_final_receipt(
    identity: &SovereignIdentity,
    command: &StoredRemoteCommand,
    outcome: &FinalOutcome,
    artifact: Option<&PreparedArtifactGrant>,
    now: i64,
) -> Result<artifact_transfer::PreparedReceipt, String> {
    let payload = serde_json::json!({
        "commandId": command.command_id,
        "remoteDeviceId": command.remote_device_id,
        "projectId": command.project_id,
        "taskRunId": command.task_run_id,
        "commandKind": command.command_kind,
        "payloadSha256": command.payload_sha256,
        "status": outcome.status,
        "outcomeCode": outcome.code,
        "contentState": artifact.map(|value| value.content_state.as_str()),
        "transferSha256": artifact.map(|value| value.transfer_sha256.as_str()),
        "completedAtMs": now,
    })
    .to_string();
    let signed = identity
        .sign_node_payload(&payload)
        .map_err(|cause| format!("remote_execution_receipt_signing_failed: {}", cause.message))?;
    Ok(artifact_transfer::PreparedReceipt {
        receipt_id: crypto::uuid_id("receipt"),
        remote_device_id: command.remote_device_id.clone(),
        command_id: command.command_id.clone(),
        receipt_kind: "remote_command".to_string(),
        payload_sha256: signed.payload_hash,
        signer_public_key: signed.public_key,
        signature: signed.signature,
        created_at_ms: now,
    })
}

fn insert_receipt(
    transaction: &Transaction<'_>,
    receipt: &artifact_transfer::PreparedReceipt,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT INTO remote_audit_receipts (receipt_id,remote_device_id,command_id,receipt_kind,payload_sha256,signature,created_at_ms,signer_public_key) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                receipt.receipt_id,
                receipt.remote_device_id,
                receipt.command_id,
                receipt.receipt_kind,
                receipt.payload_sha256,
                receipt.signature,
                receipt.created_at_ms,
                receipt.signer_public_key,
            ],
        )
        .map_err(|cause| format!("remote_execution_receipt_store_failed: {cause}"))?;
    Ok(())
}

fn result_from_persisted(
    engine: &PersistenceEngine,
    command: &StoredRemoteCommand,
    artifact: Option<&PreparedArtifactGrant>,
) -> Result<Option<RemoteCommandResult>, String> {
    let connection = engine
        .open_connection()
        .map_err(|cause| format!("remote_execution_reconcile_failed: {cause}"))?;
    let stored = command_store::load(&connection, &command.command_id)
        .map_err(|cause| cause.to_string())?
        .ok_or_else(|| {
            "remote_execution_reconcile_failed: The accepted command is missing.".to_string()
        })?;
    let receipt_exists = connection
        .query_row(
            "SELECT 1 FROM remote_audit_receipts WHERE command_id=?1 AND receipt_kind='remote_command' LIMIT 1",
            params![command.command_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|cause| format!("remote_execution_reconcile_failed: {cause}"))?
        .is_some();
    if !receipt_exists || !matches!(stored.status.as_str(), "completed" | "rejected") {
        return Ok(None);
    }
    let message = stored
        .result_json
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Your Mac completed this request.")
        .to_string();
    let task = if matches!(stored.command_kind.as_str(), "view_task" | "stop_task") {
        stored
            .task_run_id
            .as_deref()
            .map(|task_run_id| crate::tasks::get_task_for_remote(engine, task_run_id))
            .transpose()?
    } else {
        None
    };
    Ok(Some(RemoteCommandResult {
        command_id: stored.command_id,
        status: stored.status,
        outcome_code: stored
            .outcome_code
            .unwrap_or_else(|| "remote_execution_outcome_missing".to_string()),
        message,
        task,
        artifact_grant: artifact.map(PreparedArtifactGrant::response),
    }))
}

pub(crate) fn commit_sequence_conflict(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    command: &StoredRemoteCommand,
    message: &str,
) -> Result<RemoteCommandResult, String> {
    let now = crate::foundation::clock::unix_time_ms_i64();
    let outcome = FinalOutcome {
        status: "rejected",
        code: "remote_command_sequence_conflict",
        message: "This Task changed on your Mac. OOMU kept the Mac's version.",
    };
    let receipt = signed_final_receipt(identity, command, &outcome, None, now)?;
    let connection = engine
        .open_connection()
        .map_err(|cause| format!("remote_execution_store_unavailable: {cause}"))?;
    let transaction =
        rusqlite::Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
            .map_err(|cause| format!("remote_execution_begin_failed: {cause}"))?;
    let finalized = transaction
        .execute(
            "UPDATE remote_commands SET status='rejected',outcome_code=?2,result_json=?3,completed_at_ms=?4 WHERE command_id=?1 AND status='accepted'",
            params![
                command.command_id,
                outcome.code,
                serde_json::json!({"message": message}).to_string(),
                now,
            ],
        )
        .map_err(|cause| format!("remote_execution_command_finalize_failed: {cause}"))?;
    if finalized != 1 {
        drop(transaction);
        return result_from_persisted(engine, command, None)?.ok_or_else(|| {
            "remote_execution_command_unavailable: This request is no longer executable."
                .to_string()
        });
    }
    transaction
        .execute(
            "UPDATE remote_devices SET last_used_at_ms=?2 WHERE remote_device_id=?1 AND revoked_at_ms IS NULL AND expires_at_ms>=?2",
            params![command.remote_device_id, now],
        )
        .map_err(|cause| format!("remote_execution_device_update_failed: {cause}"))?;
    insert_receipt(&transaction, &receipt)?;
    transaction
        .commit()
        .map_err(|cause| format!("remote_execution_commit_failed: {cause}"))?;
    Ok(RemoteCommandResult {
        command_id: command.command_id.clone(),
        status: "rejected".to_string(),
        outcome_code: "remote_command_sequence_conflict".to_string(),
        message: message.to_string(),
        task: command
            .task_run_id
            .as_deref()
            .map(|task_run_id| crate::tasks::get_task_for_remote(engine, task_run_id))
            .transpose()?,
        artifact_grant: None,
    })
}

pub(crate) fn execute_accepted(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    signed_command: &SignedRemoteCommand,
    command: &StoredRemoteCommand,
) -> Result<RemoteCommandResult, String> {
    if let Some(existing) = result_from_persisted(engine, command, None)? {
        return Ok(existing);
    }
    let device_label = engine
        .open_connection()
        .map_err(|cause| format!("remote_execution_store_unavailable: {cause}"))?
        .query_row(
            "SELECT label FROM remote_devices WHERE remote_device_id=?1",
            params![command.remote_device_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|cause| format!("remote_execution_device_read_failed: {cause}"))?;
    let prepared_artifact = if command.command_kind == "request_artifact" {
        Some(artifact_transfer::prepare_grant(
            engine,
            identity,
            signed_command,
            &device_label,
        )?)
    } else {
        None
    };
    if let Some(grant) = prepared_artifact.as_ref() {
        artifact_transfer::revalidate_prepared(grant)?;
    }

    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|cause| format!("remote_execution_store_unavailable: {cause}"))?;
    let transaction =
        rusqlite::Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
            .map_err(|cause| format!("remote_execution_begin_failed: {cause}"))?;
    let current = command_store::load(&transaction, &command.command_id)
        .map_err(|cause| cause.to_string())?
        .ok_or_else(|| {
            "remote_execution_command_missing: The accepted request is missing.".to_string()
        })?;
    if current.status != "accepted" {
        drop(transaction);
        return result_from_persisted(engine, command, prepared_artifact.as_ref())?.ok_or_else(
            || {
                "remote_execution_command_unavailable: This request is no longer executable."
                    .to_string()
            },
        );
    }
    let device_valid = transaction
        .query_row(
            "SELECT 1 FROM remote_devices WHERE remote_device_id=?1 AND public_key=?2 AND revoked_at_ms IS NULL AND expires_at_ms>=?3",
            params![command.remote_device_id, command.signer_public_key, now],
            |_| Ok(()),
        )
        .optional()
        .map_err(|cause| format!("remote_execution_device_read_failed: {cause}"))?
        .is_some();
    if !device_valid {
        return Err(
            "remote_execution_device_unavailable: This device no longer has access.".to_string(),
        );
    }

    let mut outcome = outcome_for(&command.command_kind);
    let sequence = if let Some(task_run_id) = command.task_run_id.as_deref() {
        Some(current_sequence(&transaction, task_run_id)?)
    } else {
        None
    };
    if command.command_kind == "stop_task" && sequence != command.expected_task_sequence {
        outcome = FinalOutcome {
            status: "rejected",
            code: "remote_command_sequence_conflict",
            message: "This Task changed on your Mac. OOMU kept the Mac's version.",
        };
    } else if command.command_kind == "stop_task" {
        apply_stop_task(
            &transaction,
            command,
            sequence.expect("stop task has a sequence"),
            now,
        )?;
    }
    if outcome.status == "completed" {
        if let Some(grant) = prepared_artifact.as_ref() {
            artifact_transfer::insert_prepared(&transaction, grant)?;
        }
    }

    let result_json = serde_json::json!({
        "message": outcome.message,
        "contentState": prepared_artifact.as_ref().map(|value| value.content_state.as_str()),
        "transferSha256": prepared_artifact.as_ref().map(|value| value.transfer_sha256.as_str()),
    })
    .to_string();
    let finalized = transaction
        .execute(
            "UPDATE remote_commands SET status=?2,outcome_code=?3,result_json=?4,completed_at_ms=?5 WHERE command_id=?1 AND status='accepted'",
            params![command.command_id, outcome.status, outcome.code, result_json, now],
        )
        .map_err(|cause| format!("remote_execution_command_finalize_failed: {cause}"))?;
    if finalized != 1 {
        return Err(
            "remote_execution_command_finalize_failed: The accepted request could not be finalized."
                .to_string(),
        );
    }
    let device_updated = transaction
        .execute(
            "UPDATE remote_devices SET last_used_at_ms=?2 WHERE remote_device_id=?1 AND revoked_at_ms IS NULL AND expires_at_ms>=?2",
            params![command.remote_device_id, now],
        )
        .map_err(|cause| format!("remote_execution_device_update_failed: {cause}"))?;
    if device_updated != 1 {
        return Err(
            "remote_execution_device_update_failed: The trusted device could not be updated."
                .to_string(),
        );
    }
    let receipt =
        signed_final_receipt(identity, command, &outcome, prepared_artifact.as_ref(), now)?;
    insert_receipt(&transaction, &receipt)?;
    match transaction.commit() {
        Ok(()) => {}
        Err(cause) => {
            if let Some(result) =
                result_from_persisted(engine, command, prepared_artifact.as_ref())?
            {
                return Ok(result);
            }
            return Err(format!("remote_execution_commit_failed: {cause}"));
        }
    }
    result_from_persisted(engine, command, prepared_artifact.as_ref())?.ok_or_else(|| {
        "remote_execution_reconcile_failed: OOMU could not verify the committed result.".to_string()
    })
}
