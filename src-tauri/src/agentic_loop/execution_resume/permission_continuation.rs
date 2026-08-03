use super::*;

pub(super) struct PreparedPermissionContinuation {
    capability_id: String,
    candidate: crate::db::permission_turn_continuation::PermissionTurnContinuationCandidate,
}

pub(super) async fn prepare(
    capability_id: Option<&str>,
    persistence: &PersistenceEngine,
    execution_id: &str,
) -> Result<Option<PreparedPermissionContinuation>, AgenticLoopError> {
    let Some(capability_id) = capability_id.map(str::trim) else {
        return Ok(None);
    };
    let status = crate::macos_permission_broker::status_for_operation(capability_id).await;
    if !state_can_continue(status.state)
        || candidates(persistence, capability_id, Some(execution_id))? != [execution_id.to_string()]
    {
        return Err(error());
    }
    let candidate = persistence
        .permission_turn_continuation_candidate(execution_id)
        .map_err(agent_execution_origin_error)?;
    Ok(Some(PreparedPermissionContinuation {
        capability_id: capability_id.to_string(),
        candidate,
    }))
}

pub(super) fn record(
    continuation: PreparedPermissionContinuation,
    persistence: &PersistenceEngine,
) -> Result<(), AgenticLoopError> {
    persistence
        .prepare_permission_execution_retry(continuation.candidate, &continuation.capability_id)
        .map_err(|_| error())?;
    Ok(())
}

pub(super) fn state_can_continue(
    state: crate::macos_permission_broker::MacosPermissionState,
) -> bool {
    matches!(
        state,
        crate::macos_permission_broker::MacosPermissionState::Allowed
            | crate::macos_permission_broker::MacosPermissionState::Limited
            | crate::macos_permission_broker::MacosPermissionState::WhenUsed
    )
}

pub(super) fn candidates(
    persistence: &PersistenceEngine,
    capability_id: &str,
    requested_execution_id: Option<&str>,
) -> Result<Vec<String>, AgenticLoopError> {
    let connection = persistence
        .open_connection()
        .map_err(agent_execution_origin_error)?;
    let mut statement = connection
        .prepare(
            "SELECT executions.execution_id,
                    (SELECT logs.payload_json FROM agent_execution_logs logs
                     WHERE logs.execution_id=executions.execution_id
                       AND logs.payload_json IS NOT NULL
                     ORDER BY logs.id DESC LIMIT 1)
             FROM agent_executions executions
             WHERE executions.status='halted'
             ORDER BY executions.updated_at_ms DESC",
        )
        .map_err(agent_execution_origin_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(agent_execution_origin_error)?;
    let mut matching = Vec::new();
    for row in rows {
        let (execution_id, payload) = row.map_err(agent_execution_origin_error)?;
        if requested_execution_id.map_or(true, |requested| requested == execution_id)
            && payload
                .as_deref()
                .is_some_and(|payload| receipt_matches(payload, capability_id))
        {
            matching.push(execution_id);
        }
    }
    Ok(matching)
}

pub(super) fn receipt_matches(payload: &str, capability_id: &str) -> bool {
    let Ok(receipt) = serde_json::from_str::<serde_json::Value>(payload) else {
        return false;
    };
    if receipt.get("schema").and_then(serde_json::Value::as_str)
        != Some(recovery::RECOVERY_RECEIPT_SCHEMA)
        || receipt
            .get("recoverable")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || receipt
            .get("recoveryAction")
            .and_then(serde_json::Value::as_str)
            != Some("resume_same_execution")
    {
        return false;
    }
    if receipt
        .pointer("/context/capabilityId")
        .and_then(serde_json::Value::as_str)
        == Some(capability_id)
    {
        return true;
    }
    let code = receipt
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match capability_id {
        "calendar" => {
            code.starts_with("calendar_permission_") || code == "calendar_authorization_timeout"
        }
        "contacts" => {
            code.starts_with("contacts_permission_") || code.starts_with("contacts_authorization_")
        }
        "mail" => code == "mail_automation_permission_required",
        _ => false,
    }
}

fn error() -> AgenticLoopError {
    AgenticLoopError {
        code: "permission_continuation_not_ready",
        boundary: "ChatTurnPersistence",
        message: "This permission is not ready yet. Your saved work has not changed.".to_string(),
        mlc_path: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_authorized_native_states_can_continue_saved_work() {
        assert!(state_can_continue(
            crate::macos_permission_broker::MacosPermissionState::Allowed
        ));
        assert!(!state_can_continue(
            crate::macos_permission_broker::MacosPermissionState::Denied
        ));
    }
}
