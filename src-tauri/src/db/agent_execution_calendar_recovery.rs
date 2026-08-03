use super::*;

pub(super) struct CalendarRecoveryReceiptContext {
    pub(super) requested_calendar_name: String,
    pub(super) available_calendar_names: Vec<String>,
    pub(super) denied_arguments_sha256: Option<String>,
}

pub(super) fn receipt_context(
    payload_json: &str,
    execution_id: &str,
    plan_id: &str,
) -> Option<CalendarRecoveryReceiptContext> {
    let receipt = serde_json::from_str::<Value>(payload_json).ok()?;
    let code = receipt.get("code").and_then(Value::as_str)?;
    if receipt.get("schema").and_then(Value::as_str)
        != Some(crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA)
        || receipt.get("executionId").and_then(Value::as_str) != Some(execution_id)
        || receipt.get("planId").and_then(Value::as_str) != Some(plan_id)
        || receipt.get("recoverable").and_then(Value::as_bool) != Some(true)
        || receipt.get("recoveryAction").and_then(Value::as_str) != Some("resolve_calendar_target")
        || !matches!(
            receipt.get("changedState").and_then(Value::as_str),
            Some("none" | "checkpoint_saved")
        )
        || !matches!(
            code,
            "calendar_action_denied"
                | "calendar_not_found"
                | "calendar_name_ambiguous"
                | "calendar_read_only"
                | "calendar_availability_unsupported"
        )
    {
        return None;
    }
    let context = receipt.get("context")?.as_object()?;
    let requested = context
        .get("requestedCalendarName")?
        .as_str()?
        .trim()
        .to_string();
    if requested.is_empty() || requested.chars().count() > 80 {
        return None;
    }
    let mut available = context
        .get("availableCalendarNames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.chars().count() <= 80)
        .take(12)
        .map(str::to_string)
        .collect::<Vec<_>>();
    available.sort();
    available.dedup();
    let denied_arguments_sha256 = context
        .get("calendarStepArgumentsSha256")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase);
    if code == "calendar_action_denied" && denied_arguments_sha256.is_none() {
        return None;
    }
    Some(CalendarRecoveryReceiptContext {
        requested_calendar_name: requested,
        available_calendar_names: available,
        denied_arguments_sha256,
    })
}

pub(super) fn resolved_arguments_sha256(
    engine: &PersistenceEngine,
    connection: &Connection,
    execution_id: &str,
    plan_id: &str,
    request: &crate::agentic_loop::AgentPlanExecutionRequest,
    step_index: usize,
) -> Option<String> {
    let crate::agentic_loop::Tool::RegisteredTaskTool(planned) =
        &request.plan.steps.get(step_index)?.tool
    else {
        return None;
    };
    if !matches!(
        planned.operation.as_str(),
        "create_conflict_free_calendar_event"
            | "create_system_calendar_event"
            | "create_release_recovery_calendar_event"
    ) {
        return None;
    }
    let mut statement = connection
        .prepare(
            "SELECT output FROM actions
             WHERE plan_id=?1 AND status='completed' AND output IS NOT NULL
             ORDER BY id ASC",
        )
        .ok()?;
    let output_json = statement
        .query_map(params![plan_id], |row| row.get::<_, String>(0))
        .ok()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .ok()?;
    if output_json.len() != step_index {
        return None;
    }
    let outputs = output_json
        .iter()
        .map(|output| serde_json::from_str::<crate::shield_gate::ExecuteCommandResponse>(output))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if outputs
        .iter()
        .any(|output| !output.verified || output.status.as_str() != "completed")
    {
        return None;
    }
    let action = crate::tools::task_tool_runtime::requested_action(planned);
    let validated = crate::tools::task_tool_runtime::authorize(action).ok()?;
    let resolved =
        crate::tools::task_tool_runtime::resolve(engine, Some(execution_id), validated, &outputs)
            .ok()?;
    crate::tools::task_tool_runtime::requested_action_for_validated(&resolved)
        .content
        .as_deref()
        .map(|content| crate::foundation::digest::sha256_hex(content.as_bytes()))
}

pub(super) fn step_matches(
    request: &crate::agentic_loop::AgentPlanExecutionRequest,
    step_index: usize,
    expected_calendar_name: &str,
) -> bool {
    let Some(crate::agentic_loop::Tool::RegisteredTaskTool(tool)) =
        request.plan.steps.get(step_index).map(|step| &step.tool)
    else {
        return false;
    };
    matches!(
        tool.operation.as_str(),
        "create_conflict_free_calendar_event"
            | "create_system_calendar_event"
            | "create_release_recovery_calendar_event"
    ) && tool.arguments.get("calendarName").and_then(Value::as_str) == Some(expected_calendar_name)
}

pub(super) fn resolved_name(
    payload_json: &str,
    execution_id: &str,
    plan_id: &str,
    requested: &str,
) -> Option<String> {
    let receipt = serde_json::from_str::<Value>(payload_json).ok()?;
    if receipt.get("schema").and_then(Value::as_str)
        != Some(crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA)
        || receipt.get("executionId").and_then(Value::as_str) != Some(execution_id)
        || receipt.get("planId").and_then(Value::as_str) != Some(plan_id)
        || receipt.get("code").and_then(Value::as_str) != Some("calendar_target_resolved")
        || receipt.get("recoveryAction").and_then(Value::as_str) != Some("resume_same_execution")
        || receipt.get("recoverable").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let context = receipt.get("context")?.as_object()?;
    if context.get("requestedCalendarName").and_then(Value::as_str) != Some(requested) {
        return None;
    }
    let selected = context.get("selectedCalendarName")?.as_str()?.trim();
    (!selected.is_empty() && selected.chars().count() <= 80).then(|| selected.to_string())
}

pub(super) fn narrow_step_amendment(
    old_request: &crate::agentic_loop::AgentPlanExecutionRequest,
    new_request: &crate::agentic_loop::AgentPlanExecutionRequest,
    step_index: usize,
    selected: &str,
) -> bool {
    let (Some(old_step), Some(new_step)) = (
        old_request.plan.steps.get(step_index),
        new_request.plan.steps.get(step_index),
    ) else {
        return false;
    };
    let (Ok(mut expected), Ok(actual)) = (
        serde_json::to_value(old_step),
        serde_json::to_value(new_step),
    ) else {
        return false;
    };
    let Some(calendar_name) = expected.pointer_mut("/tool/arguments/calendarName") else {
        return false;
    };
    *calendar_name = Value::String(selected.to_string());
    expected == actual
}
