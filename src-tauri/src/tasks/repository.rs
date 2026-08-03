use super::{adapters, *};
use crate::{
    db::PersistenceEngine,
    p0_contracts::{
        EvidenceClass, P0EventEnvelope, ProjectId, TaskId, TaskRunId, TaskState,
        P0_CONTRACT_VERSION,
    },
};
use chrono::{SecondsFormat, TimeZone, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde_json::json;

fn controls(
    state: TaskState,
    runtime: &str,
    effect_verification_required: bool,
    has_unverified_effects: bool,
) -> Vec<String> {
    if effect_verification_required && matches!(state, TaskState::Blocked | TaskState::Failed) {
        return Vec::new();
    }
    match (state, runtime) {
        (TaskState::Queued, "taskflow" | "agent" | "queued_message")
        | (
            TaskState::Planning | TaskState::AwaitingApproval | TaskState::Running,
            "taskflow" | "agent",
        ) => vec!["cancel".to_string()],
        (TaskState::Blocked, "taskflow") => vec!["resume".to_string(), "cancel".to_string()],
        (TaskState::Blocked, "workflow") => vec!["resume".to_string()],
        (TaskState::Blocked, "agent") => vec!["resume".to_string(), "cancel".to_string()],
        (TaskState::Failed, _) if has_unverified_effects => {
            vec!["acknowledge_failure".to_string()]
        }
        (TaskState::Failed, "taskflow" | "workflow" | "queued_message") => {
            vec!["retry".to_string(), "acknowledge_failure".to_string()]
        }
        (TaskState::Failed, _) => vec!["acknowledge_failure".to_string()],
        _ => Vec::new(),
    }
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<TaskRunRecord> {
    let raw: String = row.get(5)?;
    let state = serde_json::from_value(json!(raw)).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let runtime: String = row.get(3)?;
    let effect_verification_required: bool = row.get(15)?;
    let has_unverified_effects: bool = row.get(16)?;
    Ok(TaskRunRecord {
        task_run_id: row.get(0)?,
        task_id: row.get(1)?,
        project_id: row.get(2)?,
        runtime_kind: runtime.clone(),
        runtime_record_id: row.get(4)?,
        state,
        origin: row.get(6)?,
        correlation_id: row.get(7)?,
        summary: row.get(8)?,
        last_error: row.get(9)?,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
        completed_at_ms: row.get(12)?,
        acknowledged_at_ms: row.get(13)?,
        recovery_state: row.get(14)?,
        effect_verification_required,
        valid_controls: controls(
            state,
            &runtime,
            effect_verification_required,
            has_unverified_effects,
        ),
    })
}

const TASK_SELECT: &str = "SELECT task_run_id, task_id, project_id, runtime_kind, runtime_record_id, state, origin, correlation_id, summary, last_error, created_at_ms, updated_at_ms, completed_at_ms, acknowledged_at_ms, recovery_state,
EXISTS(
  SELECT 1 FROM task_events required
  JOIN task_effects effect
    ON effect.task_run_id=required.task_run_id
   AND effect.idempotency_key=json_extract(required.event_json,'$.payload.idempotencyKey')
   AND effect.effect_kind=json_extract(required.event_json,'$.payload.effectKind')
  WHERE required.task_run_id=task_runs.task_run_id
    AND effect.state='executed'
    AND json_extract(required.event_json,'$.eventType')='workflow.effect.verification_required'
    AND NOT EXISTS(
      SELECT 1 FROM task_events resolved
      WHERE resolved.task_run_id=required.task_run_id
        AND resolved.sequence>required.sequence
        AND json_extract(resolved.event_json,'$.eventType')='workflow.effect.verification_resolved'
        AND json_extract(resolved.event_json,'$.payload.idempotencyKey')=effect.idempotency_key
        AND json_extract(resolved.event_json,'$.payload.effectKind')=effect.effect_kind
        AND json_extract(resolved.event_json,'$.payload.nodeId')=json_extract(required.event_json,'$.payload.nodeId')
    )
) AS effect_verification_required,
EXISTS(SELECT 1 FROM task_effects effect WHERE effect.task_run_id=task_runs.task_run_id AND effect.state!='verified') AS has_unverified_effects
FROM task_runs";

fn unresolved_effect_verification_exists(
    connection: &rusqlite::Connection,
    task_run_id: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            &format!(
                "SELECT effect_verification_required FROM ({TASK_SELECT}) WHERE task_run_id=?1"
            ),
            params![task_run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

pub(super) fn list(
    engine: &PersistenceEngine,
    filter: TaskFilter,
) -> Result<Vec<TaskRunRecord>, String> {
    let project = filter
        .project_id
        .map(ProjectId::parse)
        .transpose()?
        .map(|id| id.to_string());
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(&format!(
            "{TASK_SELECT} ORDER BY updated_at_ms DESC LIMIT 500"
        ))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], task_from_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .filter(|task| {
            project
                .as_ref()
                .is_none_or(|value| task.project_id.as_ref() == Some(value))
                && filter.state.is_none_or(|value| task.state == value)
                && filter
                    .origin
                    .as_ref()
                    .is_none_or(|value| &task.origin == value)
                && filter
                    .runtime_kind
                    .as_ref()
                    .is_none_or(|value| &task.runtime_kind == value)
                && filter
                    .from_ms
                    .is_none_or(|value| task.created_at_ms >= value)
                && filter.to_ms.is_none_or(|value| task.created_at_ms <= value)
        })
        .collect())
}

pub(crate) fn get(engine: &PersistenceEngine, raw_id: &str) -> Result<TaskRunRecord, String> {
    let id = TaskRunId::parse(raw_id)?.to_string();
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            &format!("{TASK_SELECT} WHERE task_run_id=?1"),
            params![id],
            task_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Task run was not found.".to_string())
}

fn terminal(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Completed | TaskState::Failed | TaskState::Cancelled
    )
}

fn append_event_with_sequence(
    connection: &rusqlite::Connection,
    task: &TaskRunRecord,
    event_type: &str,
    evidence: EvidenceClass,
    payload: serde_json::Value,
) -> Result<Option<u64>, String> {
    if task.project_id.is_none() {
        return Ok(None);
    }
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
    let sequence = append_event_in_transaction(&transaction, task, event_type, evidence, payload)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(sequence)
}

pub(super) fn append_event_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    task: &TaskRunRecord,
    event_type: &str,
    evidence: EvidenceClass,
    payload: serde_json::Value,
) -> Result<Option<u64>, String> {
    let Some(project_id) = task.project_id.as_deref() else {
        return Ok(None);
    };
    let sequence: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), -1) + 1 FROM task_events WHERE task_run_id=?1",
            params![task.task_run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let timestamp = Utc
        .timestamp_millis_opt(crate::foundation::clock::unix_time_ms_i64())
        .single()
        .ok_or_else(|| "Invalid task event time.".to_string())?
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let event = P0EventEnvelope {
        schema_version: P0_CONTRACT_VERSION,
        event_type: event_type.to_string(),
        project_id: ProjectId::parse(project_id)?,
        task_id: TaskId::parse(&task.task_id)?,
        task_run_id: Some(TaskRunId::parse(&task.task_run_id)?),
        correlation_id: task.correlation_id.clone(),
        sequence: sequence as u64,
        timestamp,
        evidence_class: evidence,
        payload,
    };
    let encoded = serde_json::to_string(&event).map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO task_events (task_run_id, sequence, event_json, created_at_ms) VALUES (?1, ?2, ?3, ?4)", params![task.task_run_id, sequence, encoded, crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
    Ok(Some(sequence as u64))
}

fn append_event(
    connection: &rusqlite::Connection,
    task: &TaskRunRecord,
    event_type: &str,
    evidence: EvidenceClass,
    payload: serde_json::Value,
) -> Result<(), String> {
    append_event_with_sequence(connection, task, event_type, evidence, payload).map(|_| ())
}

pub(crate) fn require_bound_task(
    engine: &PersistenceEngine,
    task_run_id: &str,
    project_id: &str,
) -> Result<TaskRunRecord, String> {
    let task = get(engine, task_run_id)?;
    if task.project_id.as_deref() != Some(project_id) {
        return Err("Task is not bound to the requested Project.".to_string());
    }
    Ok(task)
}

pub(crate) fn task_for_connector(
    engine: &PersistenceEngine,
    task_run_id: &str,
) -> Result<TaskRunRecord, String> {
    get(engine, task_run_id)
}

pub(crate) fn require_agent_runtime_task(
    engine: &PersistenceEngine,
    execution_id: &str,
) -> Result<TaskRunRecord, String> {
    reconcile_all(engine)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let workflow_binding: Option<(String, Option<String>)> = connection
        .query_row(
            "SELECT task_run_id,project_id FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            params![execution_id.trim()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((task_run_id, project_id)) = workflow_binding {
        let project_id = project_id
            .ok_or_else(|| "Scheduled Workflow Task requires a Project binding.".to_string())?;
        ProjectId::parse(&project_id)?;
        let task = get(engine, &task_run_id)?;
        if task.project_id.as_deref() != Some(project_id.as_str()) {
            return Err("Scheduled Workflow Task Project binding changed.".to_string());
        }
        return Ok(task);
    }
    let binding: Option<(String, Option<String>)> = connection
        .query_row(
            "SELECT t.task_run_id,COALESCE(a.project_id,c.project_id,t.project_id) FROM task_runs t JOIN agent_executions a ON a.execution_id=t.runtime_record_id JOIN chat_sessions c ON c.id=a.session_id WHERE t.runtime_kind='agent' AND t.runtime_record_id=?1",
            params![execution_id.trim()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let (task_run_id, project_id) =
        binding.ok_or_else(|| "connector_agent_task_not_found".to_string())?;
    let project_id = match project_id {
        Some(value) => ProjectId::parse(&value)?.to_string(),
        None => {
            let project_id =
                crate::projects::repository::ensure_internal_local_files_project(&connection)
                    .map_err(|error| error.to_string())?;
            connection
                .execute(
                    "UPDATE agent_executions SET project_id=?2,updated_at_ms=?3 WHERE execution_id=?1 AND project_id IS NULL",
                    params![
                        execution_id.trim(),
                        project_id,
                        crate::foundation::clock::unix_time_ms_i64()
                    ],
                )
                .map_err(|error| error.to_string())?;
            project_id
        }
    };
    connection
        .execute(
            "UPDATE task_runs SET project_id=?2,updated_at_ms=?3 WHERE task_run_id=?1 AND (project_id IS NULL OR project_id=?2)",
            params![
                task_run_id,
                project_id,
                crate::foundation::clock::unix_time_ms_i64()
            ],
        )
        .map_err(|error| error.to_string())?;
    let task = get(engine, &task_run_id)?;
    if task.project_id.as_deref() != Some(project_id.as_str()) {
        return Err("connector_task_binding_mismatch".to_string());
    }
    Ok(task)
}

pub(crate) fn record_domain_event(
    engine: &PersistenceEngine,
    task_run_id: &str,
    event_type: &str,
    evidence: EvidenceClass,
    payload: serde_json::Value,
) -> Result<(), String> {
    let task = get(engine, task_run_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    append_event(&connection, &task, event_type, evidence, payload)
}

pub(crate) fn record_domain_event_with_sequence(
    engine: &PersistenceEngine,
    task_run_id: &str,
    event_type: &str,
    evidence: EvidenceClass,
    payload: serde_json::Value,
) -> Result<u64, String> {
    let task = get(engine, task_run_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    append_event_with_sequence(&connection, &task, event_type, evidence, payload)?
        .ok_or_else(|| "Task event requires a Project binding.".to_string())
}

fn upsert_runtime(
    connection: &rusqlite::Connection,
    runtime: &str,
    id: &str,
    project: Option<String>,
    state: TaskState,
    origin: &str,
    summary: &str,
    error: Option<String>,
    created: i64,
    updated: i64,
) -> Result<bool, String> {
    let state_text = serde_json::to_value(state)
        .map_err(|error| error.to_string())?
        .as_str()
        .unwrap_or("failed")
        .to_string();
    let existing: Option<String> = connection
        .query_row(
            "SELECT task_run_id FROM task_runs WHERE runtime_kind=?1 AND runtime_record_id=?2",
            params![runtime, id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(run_id) = existing {
        if unresolved_effect_verification_exists(connection, &run_id)? {
            connection.execute("UPDATE task_runs SET project_id=COALESCE(?2, project_id),summary=?3,updated_at_ms=MAX(updated_at_ms,?4) WHERE task_run_id=?1", params![run_id, project, summary, updated]).map_err(|error| error.to_string())?;
            return Ok(false);
        }
        connection.execute("UPDATE task_runs SET project_id=COALESCE(?2, project_id), state=?3, summary=?4, last_error=?5, updated_at_ms=?6, completed_at_ms=CASE WHEN ?7 THEN COALESCE(completed_at_ms, ?6) ELSE NULL END, recovery_state='reconciled' WHERE task_run_id=?1", params![run_id, project, state_text, summary, error, updated, terminal(state)]).map_err(|error| error.to_string())?;
        return Ok(false);
    }
    let task_id = TaskId::new().to_string();
    let run_id = TaskRunId::new().to_string();
    connection.execute("INSERT INTO task_runs (task_run_id, task_id, project_id, runtime_kind, runtime_record_id, state, origin, correlation_id, summary, last_error, created_at_ms, updated_at_ms, completed_at_ms, recovery_state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?2, ?8, ?9, ?10, ?11, ?12, 'reconciled')", params![run_id, task_id, project, runtime, id, state_text, origin, summary, error, created, updated, terminal(state).then_some(updated)]).map_err(|error| error.to_string())?;
    Ok(true)
}

pub fn reconcile_all(engine: &PersistenceEngine) -> Result<TaskRecoveryReport, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut inspected = 0;
    let mut reconciled = 0;
    let mut ingest = |sql: &str, runtime: &str, origin: &str| -> Result<(), String> {
        let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        for (id, project, raw_state, summary, error, created, updated) in rows {
            inspected += 1;
            if upsert_runtime(
                &connection,
                runtime,
                &id,
                project,
                adapters::canonical_state(runtime, &raw_state),
                origin,
                &summary,
                error,
                created,
                updated,
            )? {
                reconciled += 1;
            }
        }
        Ok(())
    };
    ingest("SELECT flow_id, (SELECT project_id FROM chat_sessions WHERE id=taskflows.parent_session_id), status, directive, NULL, created_at_ms, updated_at_ms FROM taskflows", "taskflow", "taskflow")?;
    ingest("SELECT id, project_id, status, 'Workflow run ' || workflow_id, error_json, created_at_ms, updated_at_ms FROM execution_instances", "workflow", "workflow")?;
    ingest("SELECT execution_id, project_id, status, 'Agent execution ' || agent_id, NULL, created_at_ms, updated_at_ms FROM agent_executions", "agent", "agent")?;
    ingest("SELECT CAST(id AS TEXT), project_id, status, substr(message, 1, 240), error_message, created_at_ms, updated_at_ms FROM message_queue", "queued_message", "chat_queue")?;
    let all_tasks = list(
        engine,
        TaskFilter {
            project_id: None,
            state: None,
            origin: None,
            runtime_kind: None,
            from_ms: None,
            to_ms: None,
        },
    )?;
    for task in &all_tasks {
        let event_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM task_events WHERE task_run_id=?1",
                params![task.task_run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if event_count == 0 {
            append_event(
                &connection,
                task,
                "task.registered",
                EvidenceClass::ObservedResult,
                json!({"state": task.state, "runtime": task.runtime_kind, "origin": task.origin}),
            )?;
        }
    }
    let nonterminal = all_tasks
        .into_iter()
        .filter(|task| !terminal(task.state))
        .collect::<Vec<_>>();
    let mut lost = 0;
    let runtime_unavailable = 0;
    for task in nonterminal {
        if task.effect_verification_required {
            continue;
        }
        match adapters::runtime_state(&connection, &task.runtime_kind, &task.runtime_record_id)? {
            Some((state, error)) => {
                let state_changed = state != task.state;
                let text = serde_json::to_value(state)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string();
                connection.execute("UPDATE task_runs SET state=?2, last_error=?3, recovery_state='reconciled', updated_at_ms=?4 WHERE task_run_id=?1", params![task.task_run_id, text, error, crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
                if state_changed {
                    let updated = get(engine, &task.task_run_id)?;
                    append_event(
                        &connection,
                        &updated,
                        "task.reconciled",
                        EvidenceClass::ObservedResult,
                        json!({"previousState": task.state, "state": updated.state, "runtime": updated.runtime_kind}),
                    )?;
                }
            }
            None => {
                lost += 1;
                connection.execute("UPDATE task_runs SET state='failed', recovery_state='lost', last_error='Owning runtime record is missing.', updated_at_ms=?2 WHERE task_run_id=?1", params![task.task_run_id, crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
                let updated = get(engine, &task.task_run_id)?;
                append_event(
                    &connection,
                    &updated,
                    "task.recovery_failed",
                    EvidenceClass::ObservedResult,
                    json!({"previousState": task.state, "state": updated.state, "recoveryState": "lost"}),
                )?;
            }
        }
    }
    let cutoff = crate::foundation::clock::unix_time_ms_i64() - 90 * 24 * 60 * 60 * 1_000_i64;
    connection.execute("DELETE FROM task_runs WHERE state IN ('completed','cancelled') AND completed_at_ms < ?1 AND task_run_id NOT IN (SELECT task_run_id FROM task_effects)", params![cutoff]).map_err(|error| error.to_string())?;
    Ok(TaskRecoveryReport {
        inspected,
        reconciled,
        lost,
        runtime_unavailable,
    })
}

fn control(
    engine: &PersistenceEngine,
    raw_id: &str,
    action: &str,
) -> Result<TaskRunRecord, String> {
    engine.require_durable_store("control task")?;
    let before = get(engine, raw_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    match action {
        "cancel"
            if matches!(
                before.state,
                TaskState::Queued
                    | TaskState::Planning
                    | TaskState::AwaitingApproval
                    | TaskState::Running
                    | TaskState::Blocked
            ) =>
        {
            adapters::cancel(&connection, &before.runtime_kind, &before.runtime_record_id)?
        }
        "resume" if before.state == TaskState::Blocked => {
            adapters::resume(&connection, &before.runtime_kind, &before.runtime_record_id)?
        }
        "retry" if before.state == TaskState::Failed => {
            let unverified: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM task_effects WHERE task_run_id=?1 AND state!='verified'",
                    params![before.task_run_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if unverified > 0 {
                return Err(
                    "Retry is blocked until every prior effect has a verified postcondition."
                        .to_string(),
                );
            }
            adapters::retry(&connection, &before.runtime_kind, &before.runtime_record_id)?;
        }
        _ => return Err("Unsupported task transition.".to_string()),
    }
    reconcile_all(engine)?;
    let after = get(engine, raw_id)?;
    append_event(
        &connection,
        &after,
        &format!("task.{action}"),
        EvidenceClass::ExecutedMutation,
        json!({"previousState": before.state, "state": after.state, "runtime": after.runtime_kind}),
    )?;
    Ok(after)
}

pub(crate) fn cancel(engine: &PersistenceEngine, id: &str) -> Result<TaskRunRecord, String> {
    control(engine, id, "cancel")
}
pub(super) fn resume(engine: &PersistenceEngine, id: &str) -> Result<TaskRunRecord, String> {
    control(engine, id, "resume")
}
pub(super) fn retry(engine: &PersistenceEngine, id: &str) -> Result<TaskRunRecord, String> {
    control(engine, id, "retry")
}

pub(super) fn acknowledge(engine: &PersistenceEngine, id: &str) -> Result<TaskRunRecord, String> {
    let task = get(engine, id)?;
    if task.state != TaskState::Failed {
        return Err("Only failed work can be acknowledged.".to_string());
    }
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE task_runs SET acknowledged_at_ms=?2 WHERE task_run_id=?1",
            params![
                task.task_run_id,
                crate::foundation::clock::unix_time_ms_i64()
            ],
        )
        .map_err(|error| error.to_string())?;
    get(engine, id)
}

pub(super) fn events(
    engine: &PersistenceEngine,
    request: TaskEventsRequest,
) -> Result<Vec<P0EventEnvelope>, String> {
    let id = TaskRunId::parse(request.task_run_id)?.to_string();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection.prepare("SELECT event_json FROM task_events WHERE task_run_id=?1 AND sequence>?2 ORDER BY sequence LIMIT 500").map_err(|error| error.to_string())?;
    let encoded = statement
        .query_map(
            params![
                id,
                request
                    .after_sequence
                    .map(|value| value as i64)
                    .unwrap_or(-1)
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    encoded
        .into_iter()
        .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
        .collect()
}

pub(super) fn reserve_effect(
    engine: &PersistenceEngine,
    request: TaskEffectRequest,
) -> Result<bool, String> {
    let id = TaskRunId::parse(request.task_run_id)?.to_string();
    if request.idempotency_key.trim().is_empty() || request.effect_kind.trim().is_empty() {
        return Err("Effect identity is required.".to_string());
    }
    let changed = engine.open_connection().map_err(|error| error.to_string())?.execute("INSERT INTO task_effects (task_run_id, idempotency_key, effect_kind, state, updated_at_ms) VALUES (?1, ?2, ?3, 'reserved', ?4) ON CONFLICT(task_run_id, idempotency_key) DO NOTHING", params![id, request.idempotency_key, request.effect_kind, crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
    Ok(changed == 1)
}

pub(super) fn verify_effect(
    engine: &PersistenceEngine,
    request: TaskEffectRequest,
) -> Result<(), String> {
    let digest = request
        .result_digest
        .filter(|value| value.len() >= 16)
        .ok_or_else(|| "Verified effect requires a result digest.".to_string())?;
    let changed = engine.open_connection().map_err(|error| error.to_string())?.execute("UPDATE task_effects SET state='verified', result_digest=?3, updated_at_ms=?4 WHERE task_run_id=?1 AND idempotency_key=?2 AND state IN ('reserved','executed')", params![request.task_run_id, request.idempotency_key, digest, crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("Effect reservation was not found.".to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "project_binding_tests.rs"]
mod project_binding_tests;
