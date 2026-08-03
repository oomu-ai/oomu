use super::*;
use std::{fs, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RoutineDeliveryOutcome {
    NotRequired,
    Delivered,
    RetryableFailure { error_code: String },
    NeedsReview { error_code: String },
}

pub(super) fn routine_delivery_failure_is_safe_to_retry(error_code: &str) -> bool {
    let error_code = error_code.to_ascii_lowercase();
    ["outbound_channel_unavailable", "outbound_queue_unavailable"]
        .iter()
        .any(|candidate| error_code.contains(candidate))
}

pub(super) fn deliver_routine_notice(
    persistence: &PersistenceEngine,
    gateway: &SovereignGatewayService,
    schedule: &WorkflowScheduleRecord,
    instance_id: Option<&str>,
    status: ExecutionStatus,
    completion_kind: Option<WorkflowCompletionKind>,
    error: Option<&str>,
    approval_code: Option<&str>,
    copy: &SchedulerCopy,
    declined_actions: &[String],
) -> RoutineDeliveryOutcome {
    if !schedule.id.starts_with("routine_") {
        return RoutineDeliveryOutcome::NotRequired;
    }
    let Some(platform) = schedule
        .delivery_target
        .get("platform")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return RoutineDeliveryOutcome::NotRequired;
    };
    let Some(destination) = schedule
        .delivery_target
        .get("destination")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return RoutineDeliveryOutcome::NotRequired;
    };
    let Some((event_kind, body)) = routine_notice_copy(
        copy,
        &schedule_title(schedule),
        status,
        completion_kind,
        error,
        approval_code,
        declined_actions,
    ) else {
        return RoutineDeliveryOutcome::NotRequired;
    };
    let body = verified_routine_delivery_body(
        persistence,
        instance_id,
        status,
        &schedule_title(schedule),
        &body,
        &copy.delivery_completed_verified,
        &copy.delivery_completed_declined_verified,
        &copy.delivery_failed_verified,
        declined_actions,
    );
    let task_run_id = instance_id.and_then(|id| persistence.open_connection().ok()?.query_row("SELECT task_run_id FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1", rusqlite::params![id], |row| row.get::<_,String>(0)).optional().ok().flatten());
    let reservation = match reserve_authorized_routine_delivery(
        persistence,
        schedule,
        instance_id,
        task_run_id,
        platform,
        destination,
        event_kind,
    ) {
        Ok(RoutineDeliveryReservationState::Send(reservation)) => reservation,
        Ok(RoutineDeliveryReservationState::AlreadyDelivered) => {
            return RoutineDeliveryOutcome::Delivered;
        }
        Ok(RoutineDeliveryReservationState::NeedsReview) => {
            return RoutineDeliveryOutcome::NeedsReview {
                error_code: "routine_delivery_confirmation_required".to_string(),
            };
        }
        Err(error) => {
            let error_code = crate::redaction::redacted_log_text(&error);
            eprintln!(
                "ROUTINE_DELIVERY_RESERVATION_FAILED schedule={} event={} error={}",
                schedule.id, event_kind, error_code
            );
            // No durable receipt exists when reservation fails, so the retry
            // worker cannot safely rediscover this delivery. Surface an
            // actionable review state instead of parking a one-shot Routine in
            // an invisible retry state.
            return RoutineDeliveryOutcome::NeedsReview { error_code };
        }
    };
    if let Err(error) = mark_routine_delivery_dispatched(persistence, &reservation) {
        let error_code = crate::redaction::redacted_log_text(&error);
        // The provider has not been called yet. Convert the pending receipt to
        // the exact failed/reserved state consumed by retryable_terminal_delivery.
        // If that durable transition cannot be proven, fail closed for review.
        return if fail_routine_delivery(persistence, &reservation, &error_code, true).is_ok() {
            RoutineDeliveryOutcome::RetryableFailure { error_code }
        } else {
            RoutineDeliveryOutcome::NeedsReview { error_code }
        };
    }
    let outcome = tauri::async_runtime::block_on(gateway.deliver_routine_notice(
        persistence.clone(),
        platform,
        destination,
        &body,
    ));
    match outcome {
        Ok(provider_receipt) => {
            match finish_routine_delivery(persistence, &reservation, &provider_receipt) {
                Ok(()) => RoutineDeliveryOutcome::Delivered,
                Err(error) => {
                    let error_code = crate::redaction::redacted_log_text(&error);
                    let _ = fail_routine_delivery(persistence, &reservation, &error_code, false);
                    RoutineDeliveryOutcome::NeedsReview { error_code }
                }
            }
        }
        Err(code) => {
            let error_code = crate::redaction::redacted_log_text(&code);
            let safe_to_retry = routine_delivery_failure_is_safe_to_retry(&error_code);
            if let Err(persistence_error) =
                fail_routine_delivery(persistence, &reservation, &error_code, safe_to_retry)
            {
                eprintln!(
                    "ROUTINE_DELIVERY_FAILURE_PERSIST_FAILED schedule={} error={}",
                    schedule.id,
                    crate::redaction::redacted_log_text(&persistence_error)
                );
                return RoutineDeliveryOutcome::NeedsReview { error_code };
            }
            if safe_to_retry {
                RoutineDeliveryOutcome::RetryableFailure { error_code }
            } else {
                RoutineDeliveryOutcome::NeedsReview { error_code }
            }
        }
    }
}

pub(super) fn routine_notice_copy(
    copy: &SchedulerCopy,
    schedule_title: &str,
    status: ExecutionStatus,
    completion_kind: Option<WorkflowCompletionKind>,
    error: Option<&str>,
    approval_code: Option<&str>,
    declined_actions: &[String],
) -> Option<(&'static str, String)> {
    match status {
        ExecutionStatus::Completed
            if completion_kind == Some(WorkflowCompletionKind::EmptyCollection) =>
        {
            Some((
                "completed_empty",
                render_scheduler_copy(&copy.delivery_completed_empty, &[("name", schedule_title)]),
            ))
        }
        ExecutionStatus::Completed if !declined_actions.is_empty() => {
            let actions = declined_actions.join(", ");
            Some((
                "completed",
                render_scheduler_copy(
                    &copy.delivery_completed_declined,
                    &[("name", schedule_title), ("actions", &actions)],
                ),
            ))
        }
        ExecutionStatus::Completed => Some((
            "completed",
            render_scheduler_copy(&copy.delivery_completed, &[("name", schedule_title)]),
        )),
        ExecutionStatus::AwaitingApproval => Some((
            "blocked",
            approval_code
                .map(|code| {
                    render_scheduler_copy(
                        &copy.delivery_approval,
                        &[("name", schedule_title), ("code", code)],
                    )
                })
                .unwrap_or_else(|| {
                    render_scheduler_copy(&copy.delivery_blocked, &[("name", schedule_title)])
                }),
        )),
        ExecutionStatus::Failed => Some((
            "failed",
            render_scheduler_copy(
                &copy.delivery_failed,
                &[
                    ("name", schedule_title),
                    ("error", error.unwrap_or(&copy.delivery_repair)),
                ],
            ),
        )),
        ExecutionStatus::Pending | ExecutionStatus::Running => None,
    }
}

#[derive(Debug)]
pub(super) struct RoutineDeliveryReservation {
    pub(super) receipt_id: String,
    pub(super) task_run_id: Option<String>,
    pub(super) effect_key: Option<String>,
}

#[derive(Debug)]
pub(super) enum RoutineDeliveryReservationState {
    Send(RoutineDeliveryReservation),
    AlreadyDelivered,
    NeedsReview,
}

fn receipt_event_kind(event_kind: &str) -> &str {
    match event_kind {
        "completed_empty" => "completed",
        other => other,
    }
}

pub(super) fn reserve_routine_delivery(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    instance_id: Option<&str>,
    task_run_id: Option<String>,
    platform: &str,
    destination_hash: &str,
    event_kind: &str,
) -> Result<RoutineDeliveryReservationState, String> {
    let receipt_id = format!("delivery_{}", crate::p0_contracts::TaskId::new());
    let effect_key = task_run_id.as_ref().map(|_| {
        let transition = if event_kind == "blocked" {
            instance_id
                .and_then(|id| {
                    persistence
                        .open_connection()
                        .ok()?
                        .query_row(
                            "SELECT active_node_id FROM execution_instances WHERE id=?1",
                            rusqlite::params![id],
                            |row| row.get::<_, Option<String>>(0),
                        )
                        .optional()
                        .ok()
                        .flatten()
                        .flatten()
                })
                .unwrap_or_else(|| "approval".to_string())
        } else {
            instance_id.unwrap_or("schedule").to_string()
        };
        format!(
            "routine-delivery:{}:{}:{}",
            schedule.id, event_kind, transition
        )
    });
    let now = unix_time_ms();
    let mut connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    if let (Some(task_run_id), Some(effect_key)) = (&task_run_id, &effect_key) {
        let reserved = transaction
            .execute(
                "INSERT INTO task_effects (task_run_id,idempotency_key,effect_kind,state,updated_at_ms) VALUES (?1,?2,'routine_channel_delivery','reserved',?3) ON CONFLICT(task_run_id,idempotency_key) DO NOTHING",
                rusqlite::params![task_run_id, effect_key, now],
            )
            .map_err(|error| error.to_string())?;
        if reserved == 0 {
            let effect_state: String = transaction
                .query_row(
                    "SELECT state FROM task_effects WHERE task_run_id=?1 AND idempotency_key=?2 AND effect_kind='routine_channel_delivery'",
                    rusqlite::params![task_run_id, effect_key],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let receipt: Option<(String, String)> = transaction
                .query_row(
                    "SELECT receipt_id,state FROM routine_delivery_receipts WHERE schedule_id=?1 AND task_run_id=?2 AND event_kind=?3 ORDER BY created_at_ms DESC LIMIT 1",
                    rusqlite::params![schedule.id, task_run_id, receipt_event_kind(event_kind)],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if effect_state == "verified"
                || receipt
                    .as_ref()
                    .is_some_and(|(_, state)| state == "delivered")
            {
                if effect_state != "verified" {
                    transaction
                        .execute(
                            "UPDATE task_effects SET state='verified',updated_at_ms=?3 WHERE task_run_id=?1 AND idempotency_key=?2 AND state IN ('reserved','executed')",
                            rusqlite::params![task_run_id, effect_key, now],
                        )
                        .map_err(|error| error.to_string())?;
                }
                transaction.commit().map_err(|error| error.to_string())?;
                return Ok(RoutineDeliveryReservationState::AlreadyDelivered);
            }
            if effect_state == "executed" {
                transaction.commit().map_err(|error| error.to_string())?;
                return Ok(RoutineDeliveryReservationState::NeedsReview);
            }
            if effect_state != "reserved" {
                return Err("Routine delivery reservation is in an invalid state.".to_string());
            }
            if let Some((existing_receipt_id, _)) = receipt {
                transaction
                    .execute(
                        "UPDATE routine_delivery_receipts SET state='pending',error_code=NULL,updated_at_ms=?2 WHERE receipt_id=?1",
                        rusqlite::params![existing_receipt_id, now],
                    )
                    .map_err(|error| error.to_string())?;
                transaction.commit().map_err(|error| error.to_string())?;
                return Ok(RoutineDeliveryReservationState::Send(
                    RoutineDeliveryReservation {
                        receipt_id: existing_receipt_id,
                        task_run_id: Some(task_run_id.clone()),
                        effect_key: Some(effect_key.clone()),
                    },
                ));
            }
        }
    }
    transaction
        .execute(
            "INSERT INTO routine_delivery_receipts (receipt_id,schedule_id,task_run_id,platform,destination_hash,event_kind,state,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,?5,?6,'pending',?7,?7)",
            rusqlite::params![receipt_id,schedule.id,task_run_id,platform,destination_hash,receipt_event_kind(event_kind),now],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(RoutineDeliveryReservationState::Send(
        RoutineDeliveryReservation {
            receipt_id,
            task_run_id,
            effect_key,
        },
    ))
}

pub(super) fn reserve_authorized_routine_delivery(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    instance_id: Option<&str>,
    task_run_id: Option<String>,
    platform: &str,
    destination: &str,
    event_kind: &str,
) -> Result<RoutineDeliveryReservationState, String> {
    let instance_id = instance_id.ok_or_else(|| {
        "Routine terminal delivery requires an exact Workflow instance.".to_string()
    })?;
    let arguments = json!({"platform":platform,"destination":destination});
    let authorized = crate::routines::verify_reviewed_workflow_scope(
        persistence,
        instance_id,
        crate::routines::TERMINAL_DELIVERY_NODE_ID,
        crate::routines::TERMINAL_DELIVERY_TOOL,
        &arguments,
    )?;
    if !authorized {
        return Err("Routine terminal delivery authority no longer matches.".to_string());
    }
    let destination_hash =
        crate::foundation::digest::sha256_hex(format!("{platform}:{destination}").as_bytes());
    reserve_routine_delivery(
        persistence,
        schedule,
        Some(instance_id),
        task_run_id,
        platform,
        &destination_hash,
        event_kind,
    )
}

pub(super) fn mark_routine_delivery_dispatched(
    persistence: &PersistenceEngine,
    reservation: &RoutineDeliveryReservation,
) -> Result<(), String> {
    let (Some(task_run_id), Some(effect_key)) = (
        reservation.task_run_id.as_deref(),
        reservation.effect_key.as_deref(),
    ) else {
        return Ok(());
    };
    let changed = persistence
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE task_effects SET state='executed',updated_at_ms=?3 WHERE task_run_id=?1 AND idempotency_key=?2 AND state='reserved'",
            rusqlite::params![task_run_id, effect_key, unix_time_ms()],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Routine delivery could not enter its durable dispatch boundary.".to_string());
    }
    Ok(())
}

pub(super) fn finish_routine_delivery(
    persistence: &PersistenceEngine,
    reservation: &RoutineDeliveryReservation,
    provider_receipt: &str,
) -> Result<(), String> {
    let digest = crate::foundation::digest::sha256_hex(provider_receipt.as_bytes());
    let now = unix_time_ms();
    let mut connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let receipt_changed = transaction
        .execute(
            "UPDATE routine_delivery_receipts SET state='delivered',provider_receipt_hash=?2,error_code=NULL,updated_at_ms=?3 WHERE receipt_id=?1 AND state='pending'",
            rusqlite::params![reservation.receipt_id, digest, now],
        )
        .map_err(|error| error.to_string())?;
    if receipt_changed != 1 {
        return Err("Routine delivery receipt could not be verified durably.".to_string());
    }
    if let (Some(task_run_id), Some(effect_key)) = (
        reservation.task_run_id.as_deref(),
        reservation.effect_key.as_deref(),
    ) {
        let effect_changed = transaction
            .execute(
                "UPDATE task_effects SET state='verified',result_digest=?3,updated_at_ms=?4 WHERE task_run_id=?1 AND idempotency_key=?2 AND state='executed'",
                rusqlite::params![task_run_id, effect_key, digest, now],
            )
            .map_err(|error| error.to_string())?;
        if effect_changed != 1 {
            return Err("Routine delivery effect could not be verified durably.".to_string());
        }
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn fail_routine_delivery(
    persistence: &PersistenceEngine,
    reservation: &RoutineDeliveryReservation,
    error_code: &str,
    safe_to_retry: bool,
) -> Result<(), String> {
    let now = unix_time_ms();
    let mut connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE routine_delivery_receipts SET state='failed',error_code=?2,updated_at_ms=?3 WHERE receipt_id=?1 AND state='pending'",
            rusqlite::params![reservation.receipt_id, error_code, now],
        )
        .map_err(|error| error.to_string())?;
    if safe_to_retry {
        if let (Some(task_run_id), Some(effect_key)) = (
            reservation.task_run_id.as_deref(),
            reservation.effect_key.as_deref(),
        ) {
            transaction
                .execute(
                    "UPDATE task_effects SET state='reserved',updated_at_ms=?3 WHERE task_run_id=?1 AND idempotency_key=?2 AND state='executed'",
                    rusqlite::params![task_run_id, effect_key, now],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn retryable_terminal_delivery(
    persistence: &PersistenceEngine,
    now: i64,
    retry_backoff_ms: i64,
) -> Result<Option<(WorkflowScheduleRecord, String)>, String> {
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let candidate: Option<(String, String)> = connection
        .query_row(
            "SELECT d.schedule_id,r.execution_instance_id FROM routine_delivery_receipts d JOIN routine_runs r ON r.schedule_id=d.schedule_id AND r.task_run_id=d.task_run_id JOIN task_effects e ON e.task_run_id=d.task_run_id AND e.effect_kind='routine_channel_delivery' AND e.idempotency_key LIKE 'routine-delivery:' || d.schedule_id || ':completed%:%' JOIN workflow_schedules s ON s.id=d.schedule_id WHERE d.event_kind='completed' AND d.state='failed' AND e.state='reserved' AND s.schedule_kind='one_shot' AND (d.error_code='routine_delivery_absence_confirmed' OR d.updated_at_ms<=?1) ORDER BY d.updated_at_ms ASC LIMIT 1",
            rusqlite::params![now.saturating_sub(retry_backoff_ms)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    drop(connection);
    let Some((schedule_id, instance_id)) = candidate else {
        return Ok(None);
    };
    let schedule = persistence
        .load_workflow_schedule(&schedule_id)
        .map_err(|error| error.to_string())?;
    Ok(Some((schedule, instance_id)))
}

pub(crate) fn confirm_terminal_delivery_absent_and_retry(
    persistence: &PersistenceEngine,
    schedule_id: &str,
) -> Result<(), String> {
    let now = unix_time_ms();
    let mut connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let candidate: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT d.receipt_id,e.task_run_id,e.idempotency_key FROM routine_delivery_receipts d JOIN task_effects e ON e.task_run_id=d.task_run_id AND e.effect_kind='routine_channel_delivery' AND e.idempotency_key LIKE 'routine-delivery:' || d.schedule_id || ':completed%:%' WHERE d.schedule_id=?1 AND d.event_kind='completed' AND d.state IN ('pending','failed') AND e.state='executed' ORDER BY d.created_at_ms DESC LIMIT 1",
            rusqlite::params![schedule_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((receipt_id, task_run_id, effect_key)) = candidate else {
        return Err("No uncertain Routine delivery is waiting for confirmation.".to_string());
    };
    transaction
        .execute(
            "UPDATE task_effects SET state='reserved',updated_at_ms=?3 WHERE task_run_id=?1 AND idempotency_key=?2 AND state='executed'",
            rusqlite::params![task_run_id, effect_key, now],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE routine_delivery_receipts SET state='failed',updated_at_ms=?2,error_code='routine_delivery_absence_confirmed' WHERE receipt_id=?1 AND state IN ('pending','failed')",
            rusqlite::params![receipt_id, now],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE workflow_schedules SET is_active=1,paused_reason='Routine delivery retry pending',last_status='Pending',last_error=NULL,updated_at_ms=?2 WHERE id=?1 AND schedule_kind='one_shot'",
            rusqlite::params![schedule_id, now],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

fn verified_artifact_names(persistence: &PersistenceEngine, instance_id: &str) -> Vec<String> {
    let Ok(connection) = persistence.open_connection() else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT e.event_json FROM task_events e JOIN task_runs t ON t.task_run_id=e.task_run_id WHERE t.runtime_kind='workflow' AND t.runtime_record_id=?1 ORDER BY e.sequence",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map(rusqlite::params![instance_id], |row| {
        row.get::<_, String>(0)
    }) else {
        return Vec::new();
    };
    let mut names = rows
        .filter_map(Result::ok)
        .filter_map(|encoded| serde_json::from_str::<Value>(&encoded).ok())
        .filter(|event| event.get("eventType").and_then(Value::as_str) == Some("file.created"))
        .filter(|event| {
            event.get("evidenceClass").and_then(Value::as_str) == Some("verified_postcondition")
        })
        .filter_map(|event| {
            let payload = event.get("payload")?;
            let path = payload.get("path").and_then(Value::as_str)?;
            let expected_sha = payload.get("sha256").and_then(Value::as_str)?;
            let expected_bytes = payload.get("byteLength").and_then(Value::as_u64)?;
            let metadata = fs::symlink_metadata(path).ok()?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() != expected_bytes
            {
                return None;
            }
            let bytes = fs::read(path).ok()?;
            if crate::foundation::digest::sha256_hex(&bytes) != expected_sha {
                return None;
            }
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names.truncate(20);
    names
}

pub(super) fn verified_routine_delivery_body(
    persistence: &PersistenceEngine,
    instance_id: Option<&str>,
    status: ExecutionStatus,
    schedule_title: &str,
    fallback: &str,
    completed_template: &str,
    completed_declined_template: &str,
    failed_template: &str,
    declined_actions: &[String],
) -> String {
    let Some(instance_id) = instance_id else {
        return fallback.to_string();
    };
    let artifacts = verified_artifact_names(persistence, instance_id);
    if artifacts.is_empty() {
        return fallback.to_string();
    }
    let filenames = artifacts.join(", ");
    match status {
        ExecutionStatus::Completed if !declined_actions.is_empty() => {
            let actions = declined_actions.join(", ");
            render_scheduler_copy(
                completed_declined_template,
                &[
                    ("name", schedule_title),
                    ("filenames", &filenames),
                    ("actions", &actions),
                ],
            )
        }
        ExecutionStatus::Completed => render_scheduler_copy(
            completed_template,
            &[("name", schedule_title), ("filenames", &filenames)],
        ),
        ExecutionStatus::Failed => render_scheduler_copy(
            failed_template,
            &[("fallback", fallback), ("filenames", &filenames)],
        ),
        _ => fallback.to_string(),
    }
}
