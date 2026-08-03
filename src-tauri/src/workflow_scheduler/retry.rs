use super::*;

const TRANSIENT_RETRY_BACKOFF_MS: i64 = 30_000;

fn transient_retry_waiting_copy(persistence: &PersistenceEngine) -> String {
    settings::locale_state_for_engine(persistence, None)
        .ok()
        .and_then(|state| {
            state
                .translations
                .pointer("/workflow_scheduler/retry/waiting")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Waiting for the connection to return.".to_string())
}

pub(super) fn retryable_instance_for_claim(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
) -> Result<Option<String>, String> {
    let Some(instance_id) = schedule.last_instance_id.as_deref() else {
        return Ok(None);
    };
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let retryable = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM execution_instances e JOIN routine_runs r ON r.execution_instance_id=e.id WHERE e.id=?1 AND e.status='Pending' AND r.schedule_id=?2)",
            rusqlite::params![instance_id, schedule.id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| error.to_string())?;
    if !retryable {
        return Ok(None);
    }
    connection
        .execute(
            "UPDATE task_runs SET state='running',last_error=NULL,updated_at_ms=?2,recovery_state='reconciled' WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance_id, unix_time_ms()],
        )
        .map_err(|error| error.to_string())?;
    Ok(Some(instance_id.to_string()))
}

fn transient_network_failure(response: &workflow_runtime::RunWorkflowResponse) -> bool {
    if response.instance.status != ExecutionStatus::Failed {
        return false;
    }
    let error = response
        .instance
        .error
        .as_ref()
        .cloned()
        .unwrap_or(Value::Null);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stable_code = [
        "network_unavailable",
        "dns_resolution_failed",
        "network_timeout",
        "connection_failed",
    ]
    .iter()
    .any(|candidate| code.contains(candidate) || message.contains(candidate));
    let network_context = [
        "network",
        "connection",
        "dns",
        "search_web",
        "fetch_official_page",
        "http",
    ]
    .iter()
    .any(|candidate| message.contains(candidate));
    let transient_phrase = [
        "network is unreachable",
        "not connected to the internet",
        "temporary failure in name resolution",
        "name or service not known",
        "failed to lookup address",
        "connection refused",
        "connection reset",
        "timed out",
        "timeout",
    ]
    .iter()
    .any(|candidate| message.contains(candidate));
    stable_code || (network_context && transient_phrase)
}

fn task_has_effects(persistence: &PersistenceEngine, instance_id: &str) -> Result<bool, String> {
    persistence
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM task_effects e JOIN task_runs t ON t.task_run_id=e.task_run_id WHERE t.runtime_kind='workflow' AND t.runtime_record_id=?1)",
            rusqlite::params![instance_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

pub(super) fn requeue_transient_failure(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    response: &workflow_runtime::RunWorkflowResponse,
) -> Result<bool, String> {
    if !transient_network_failure(response) || task_has_effects(persistence, &response.instance.id)?
    {
        return Ok(false);
    }
    let waiting = transient_retry_waiting_copy(persistence);
    let retry_at = unix_time_ms().saturating_add(TRANSIENT_RETRY_BACKOFF_MS);
    let mut connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let paused = transaction
        .execute(
            "UPDATE execution_instances SET status='Pending',active_node_id=NULL,completed_at_ms=NULL,updated_at_ms=?2 WHERE id=?1 AND status='Failed'",
            rusqlite::params![response.instance.id, unix_time_ms()],
        )
        .map_err(|error| error.to_string())?;
    if paused != 1 {
        return Ok(false);
    }
    transaction
        .execute(
            "UPDATE task_runs SET state='blocked',last_error=?2,updated_at_ms=?3,recovery_state='recoverable' WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![response.instance.id, waiting.as_str(), unix_time_ms()],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE workflow_schedules SET is_active=1,claimed_at_ms=NULL,next_run_at_ms=?2,last_status='Pending',last_error=?3,last_instance_id=?4,updated_at_ms=?5 WHERE id=?1",
            rusqlite::params![schedule.id,retry_at,waiting.as_str(),response.instance.id,unix_time_ms()],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(true)
}
