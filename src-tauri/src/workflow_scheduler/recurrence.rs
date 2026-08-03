use super::*;

const FINISH_ROUTINE_SQL: &str = concat!(
    "UPDATE workflow_schedules ",
    "SET is_active=0,claimed_at_ms=NULL,next_run_at_ms=NULL,",
    "paused_reason='Routine end time reached',updated_at_ms=?2 WHERE id=?1"
);
const PERSIST_SCHEDULE_RESULT_SQL: &str = concat!(
    "UPDATE workflow_schedules SET claimed_at_ms=NULL,",
    "last_completed_at_ms=CASE WHEN ?3 IN ('Completed','Failed') ",
    "THEN ?2 ELSE last_completed_at_ms END,last_status=?3,last_error=?4,",
    "last_instance_id=?5,next_run_at_ms=?6,run_request_json=?7,",
    "updated_at_ms=?2 WHERE id=?1"
);

#[derive(Clone, Debug)]
pub(super) struct ClaimedOccurrencePlan {
    pub(super) schedule: WorkflowScheduleRecord,
    pub(super) next_run_at_ms: Option<i64>,
}

fn schedule_candidate_after(
    schedule: &WorkflowScheduleRecord,
    after_ms: i64,
) -> Result<i64, String> {
    if schedule.id.starts_with("routine_") {
        return next_run_after_in_timezone(
            &schedule.schedule_expression,
            &schedule.routine_timezone,
            after_ms,
        );
    }
    next_run_after(&schedule.schedule_expression, after_ms)
}

fn candidate_within_end_boundary(
    schedule: &WorkflowScheduleRecord,
    candidate: i64,
) -> Result<Option<i64>, String> {
    let ended = crate::routines::control::end_at_ms(&schedule.run_request)?
        .is_some_and(|end_at_ms| candidate >= end_at_ms);
    Ok((!ended).then_some(candidate))
}

pub(super) fn next_run_with_end_boundary(
    schedule: &WorkflowScheduleRecord,
    after_ms: i64,
) -> Result<Option<i64>, String> {
    if schedule.schedule_kind == "one_shot" {
        return Ok(None);
    }
    candidate_within_end_boundary(schedule, schedule_candidate_after(schedule, after_ms)?)
}

pub(super) fn finish_routine_at_end_boundary(
    persistence: &PersistenceEngine,
    schedule_id: &str,
) -> Result<(), String> {
    persistence
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            FINISH_ROUTINE_SQL,
            rusqlite::params![schedule_id, unix_time_ms()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn finalize_due_routines_at_end_boundary(
    persistence: &PersistenceEngine,
    now: i64,
) -> Result<(), String> {
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT id,run_request_json FROM workflow_schedules WHERE id LIKE 'routine_%' AND is_active=1 AND claimed_at_ms IS NULL",
        )
        .map_err(|error| error.to_string())?;
    let routines = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    drop(connection);

    for (schedule_id, run_request) in routines {
        let run_request =
            serde_json::from_str::<Value>(&run_request).map_err(|error| error.to_string())?;
        if crate::routines::control::end_at_ms(&run_request)?
            .is_some_and(|end_at_ms| now >= end_at_ms)
        {
            finish_routine_at_end_boundary(persistence, &schedule_id)?;
        }
    }
    Ok(())
}

fn fixed_interval_ms(expression: &str) -> Option<i64> {
    let normalized = expression.trim().to_ascii_lowercase();
    if normalized == "hourly" {
        return Some(60 * 60 * 1_000);
    }
    let parts = normalized
        .strip_prefix("every ")?
        .split_whitespace()
        .collect::<Vec<_>>();
    let (amount, unit) = match parts.as_slice() {
        [unit] => (1_i64, *unit),
        [amount, unit] => (amount.parse::<i64>().ok()?, *unit),
        _ => return None,
    };
    let unit_ms = match unit.trim_end_matches('s') {
        "minute" | "min" => 60 * 1_000,
        "hour" | "hr" => 60 * 60 * 1_000,
        _ => return None,
    };
    amount.checked_mul(unit_ms).filter(|interval| *interval > 0)
}

pub(super) fn first_run_after_now_from_occurrence(
    schedule: &WorkflowScheduleRecord,
    occurrence_ms: i64,
    now: i64,
) -> Result<Option<i64>, String> {
    let Some(mut candidate) = next_run_with_end_boundary(schedule, occurrence_ms)? else {
        return Ok(None);
    };
    if candidate > now {
        return Ok(Some(candidate));
    }
    if let Some(interval_ms) = fixed_interval_ms(&schedule.schedule_expression) {
        let skipped = now.saturating_sub(candidate) / interval_ms + 1;
        candidate = candidate
            .checked_add(skipped.saturating_mul(interval_ms))
            .ok_or_else(|| "Routine recurrence is out of range.".to_string())?;
        return candidate_within_end_boundary(schedule, candidate);
    }
    while candidate <= now {
        let previous = candidate;
        let Some(next) = next_run_with_end_boundary(schedule, candidate)? else {
            return Ok(None);
        };
        if next <= previous {
            return Err("Routine recurrence did not advance.".to_string());
        }
        candidate = next;
    }
    Ok(Some(candidate))
}

fn planned_next_after_claim(
    schedule: &WorkflowScheduleRecord,
    now: i64,
) -> Result<Option<i64>, String> {
    if schedule.schedule_kind == "one_shot" {
        return Ok(None);
    }
    if let Some(resume_at_ms) =
        crate::routines::control::run_now_resume_at_ms(&schedule.run_request)?
    {
        return candidate_within_end_boundary(schedule, resume_at_ms);
    }
    let occurrence_ms = schedule
        .next_run_at_ms
        .ok_or_else(|| "Claimed Routine has no scheduled occurrence.".to_string())?;
    if schedule.id.starts_with("routine_") && schedule.missed_run_policy != "run_each" {
        first_run_after_now_from_occurrence(schedule, occurrence_ms, now)
    } else {
        next_run_with_end_boundary(schedule, occurrence_ms)
    }
}

pub(super) fn claimed_occurrences_at(
    schedule: &WorkflowScheduleRecord,
    now: i64,
) -> Result<Vec<ClaimedOccurrencePlan>, String> {
    if crate::routines::control::run_now_resume_at_ms(&schedule.run_request)?.is_some()
        || !schedule.id.starts_with("routine_")
        || schedule.missed_run_policy != "run_each"
    {
        return Ok(vec![ClaimedOccurrencePlan {
            schedule: schedule.clone(),
            next_run_at_ms: planned_next_after_claim(schedule, now)?,
        }]);
    }

    let mut due = schedule.next_run_at_ms.unwrap_or(now);
    let mut occurrences = Vec::new();
    while due <= now && occurrences.len() < usize::from(schedule.missed_run_cap.max(1)) {
        let mut occurrence = schedule.clone();
        occurrence.next_run_at_ms = Some(due);
        let next_run_at_ms = next_run_with_end_boundary(schedule, due)?;
        occurrences.push(ClaimedOccurrencePlan {
            schedule: occurrence,
            next_run_at_ms,
        });
        let Some(next) = next_run_at_ms else {
            break;
        };
        if next <= due {
            return Err("Routine recurrence did not advance.".to_string());
        }
        due = next;
    }

    if occurrences.is_empty() {
        occurrences.push(ClaimedOccurrencePlan {
            schedule: schedule.clone(),
            next_run_at_ms: planned_next_after_claim(schedule, now)?,
        });
    } else if occurrences
        .last()
        .and_then(|occurrence| occurrence.next_run_at_ms)
        .is_some_and(|next| next <= now)
    {
        let last = occurrences.last_mut().expect("non-empty occurrence plan");
        let occurrence_ms = last.schedule.next_run_at_ms.unwrap_or(now);
        last.next_run_at_ms = first_run_after_now_from_occurrence(schedule, occurrence_ms, now)?;
    }
    Ok(occurrences)
}

pub(super) fn release_claimed_at_next_future_occurrence(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    now: i64,
    reason: &str,
) -> Result<(), String> {
    let occurrence_ms = schedule.next_run_at_ms.unwrap_or(now);
    match first_run_after_now_from_occurrence(schedule, occurrence_ms, now)? {
        Some(next) => release_claimed_without_run(persistence, &schedule.id, next, reason),
        None => finish_routine_at_end_boundary(persistence, &schedule.id),
    }
}

fn execution_status_label(status: ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Pending => "Pending",
        ExecutionStatus::Running => "Running",
        ExecutionStatus::AwaitingApproval => "AwaitingApproval",
        ExecutionStatus::Completed => "Completed",
        ExecutionStatus::Failed => "Failed",
    }
}

fn run_request_after_run_now(schedule: &WorkflowScheduleRecord) -> Result<Value, String> {
    if crate::routines::control::run_now_resume_at_ms(&schedule.run_request)?.is_some() {
        crate::routines::control::without_run_now_resume(&schedule.run_request)
    } else {
        Ok(schedule.run_request.clone())
    }
}

pub(super) fn persist_claimed_schedule_result(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    status: ExecutionStatus,
    instance_id: Option<&str>,
    error_message: Option<&str>,
    next_run_at_ms: Option<i64>,
) -> Result<(), String> {
    let run_request = run_request_after_run_now(schedule)?;
    let now = unix_time_ms();
    let mut connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let changed = transaction
        .execute(
            PERSIST_SCHEDULE_RESULT_SQL,
            rusqlite::params![
                schedule.id,
                now,
                execution_status_label(status),
                error_message.map(str::trim),
                instance_id.map(str::trim),
                next_run_at_ms,
                run_request.to_string(),
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(format!(
            "Workflow schedule {} disappeared before its result could be persisted.",
            schedule.id
        ));
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn advance_if_recorded(
    persistence: &PersistenceEngine,
    occurrence: &ClaimedOccurrencePlan,
) -> Result<bool, String> {
    if !occurrence.schedule.id.starts_with("routine_") {
        return Ok(false);
    }
    let Some(scheduled_for_ms) = occurrence.schedule.next_run_at_ms else {
        return Ok(false);
    };
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let recorded = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM routine_runs WHERE schedule_id=?1 AND scheduled_for_ms=?2)",
            rusqlite::params![occurrence.schedule.id, scheduled_for_ms],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| error.to_string())?;
    if !recorded {
        return Ok(false);
    }

    let run_request = run_request_after_run_now(&occurrence.schedule)?;
    let changed = connection
        .execute(
            "UPDATE workflow_schedules SET claimed_at_ms=NULL,next_run_at_ms=?2,run_request_json=?3,updated_at_ms=?4 WHERE id=?1",
            rusqlite::params![
                occurrence.schedule.id,
                occurrence.next_run_at_ms,
                run_request.to_string(),
                unix_time_ms(),
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Recorded Routine occurrence could not be advanced.".to_string());
    }
    Ok(true)
}
