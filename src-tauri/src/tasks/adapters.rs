use crate::p0_contracts::TaskState;
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn canonical_state(runtime: &str, state: &str) -> TaskState {
    match (runtime, state.to_ascii_lowercase().as_str()) {
        (_, "queued" | "pending") => TaskState::Queued,
        (_, "planning") => TaskState::Planning,
        (_, "awaitingapproval" | "awaiting_approval" | "secure_pause") => {
            TaskState::AwaitingApproval
        }
        (_, "running" | "active" | "processing") => TaskState::Running,
        (_, "paused" | "diagnostic" | "halted") => TaskState::Blocked,
        (_, "completed" | "verified") => TaskState::Completed,
        (_, "cancelled") => TaskState::Cancelled,
        _ => TaskState::Failed,
    }
}

pub(super) fn runtime_state(
    connection: &Connection,
    runtime: &str,
    id: &str,
) -> Result<Option<(TaskState, Option<String>)>, String> {
    let query = match runtime {
        "taskflow" => "SELECT status, NULL FROM taskflows WHERE flow_id=?1",
        "workflow" => "SELECT status, error_json FROM execution_instances WHERE id=?1",
        "agent" => {
            "SELECT status,CASE WHEN status IN ('failed','halted') THEN
                    (SELECT message FROM agent_execution_logs logs
                     WHERE logs.execution_id=agent_executions.execution_id
                     ORDER BY logs.id DESC LIMIT 1) ELSE NULL END
                    FROM agent_executions WHERE execution_id=?1"
        }
        "queued_message" => {
            "SELECT status, error_message FROM message_queue WHERE CAST(id AS TEXT)=?1"
        }
        _ => return Ok(None),
    };
    connection
        .query_row(query, params![id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .optional()
        .map_err(|error| error.to_string())
        .map(|value| value.map(|(state, error)| (canonical_state(runtime, &state), error)))
}

pub(super) fn cancel(connection: &Connection, runtime: &str, id: &str) -> Result<(), String> {
    let changed = match runtime {
        "taskflow" => {
            connection.execute("UPDATE taskflow_steps SET status='cancelled' WHERE flow_id=?1 AND status IN ('queued','active')", params![id]).map_err(|error| error.to_string())?;
            connection.execute("UPDATE taskflows SET status='cancelled', updated_at_ms=?2 WHERE flow_id=?1 AND status NOT IN ('verified','cancelled')", params![id, crate::foundation::clock::unix_time_ms_i64()])
        }
        "agent" => connection.execute("UPDATE agent_executions SET status='cancelled', updated_at_ms=?2 WHERE execution_id=?1 AND status IN ('running','halted')", params![id, crate::foundation::clock::unix_time_ms_i64()]),
        "queued_message" => connection.execute("UPDATE message_queue SET status='cancelled', updated_at_ms=?2, error_message='Cancelled by user.' WHERE CAST(id AS TEXT)=?1 AND status='queued'", params![id, crate::foundation::clock::unix_time_ms_i64()]),
        _ => return Err("This runtime does not support cancellation at its current safe boundary.".to_string()),
    }.map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The owning runtime rejected cancellation.".to_string());
    }
    Ok(())
}

pub(super) fn resume(connection: &Connection, runtime: &str, id: &str) -> Result<(), String> {
    let now = crate::foundation::clock::unix_time_ms_i64();
    let changed = match runtime {
        "taskflow" => connection.execute("UPDATE taskflows SET status='queued', updated_at_ms=?2 WHERE flow_id=?1 AND status IN ('paused','diagnostic')", params![id, now]),
        "workflow" => connection.execute("UPDATE execution_instances SET status='Pending', updated_at_ms=?2 WHERE id=?1 AND status='Paused'", params![id, now]),
        _ => return Err("This runtime does not support resume.".to_string()),
    }.map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The owning runtime rejected resume.".to_string());
    }
    Ok(())
}

pub(super) fn retry(connection: &Connection, runtime: &str, id: &str) -> Result<(), String> {
    let now = crate::foundation::clock::unix_time_ms_i64();
    let changed = match runtime {
        "taskflow" => connection.execute("UPDATE taskflows SET status='queued', updated_at_ms=?2 WHERE flow_id=?1 AND status IN ('failed','diagnostic')", params![id, now]),
        "workflow" => connection.execute("UPDATE execution_instances SET status='Pending', active_node_id=NULL, error_json=NULL, updated_at_ms=?2 WHERE id=?1 AND status='Failed'", params![id, now]),
        "queued_message" => connection.execute("UPDATE message_queue SET status='queued', error_message=NULL, updated_at_ms=?2 WHERE CAST(id AS TEXT)=?1 AND status IN ('failed','cancelled')", params![id, now]),
        _ => return Err("This runtime cannot safely reconstruct a retry.".to_string()),
    }.map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The owning runtime rejected retry.".to_string());
    }
    Ok(())
}
