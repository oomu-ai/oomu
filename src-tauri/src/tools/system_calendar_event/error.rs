use super::CalendarReadFailure;
use serde_json::{json, Value};

pub(crate) fn calendar_failure_error(failure: CalendarReadFailure) -> String {
    let target_resolution_required = matches!(
        failure.code.as_str(),
        "calendar_not_found"
            | "calendar_name_ambiguous"
            | "calendar_read_only"
            | "calendar_availability_unsupported"
    );
    let verified_unchanged = target_resolution_required
        || matches!(
            failure.code.as_str(),
            "calendar_permission_denied"
                | "calendar_permission_restricted"
                | "calendar_permission_write_only"
                | "calendar_permission_unavailable"
                | "calendar_authorization_timeout"
                | "calendar_authorization_failed"
                | "calendar_authorization_interrupted"
                | "calendar_access_check_timeout"
                | "calendar_store_reset_timeout"
                | "calendar_source_refresh_timeout"
                | "calendar_operation_busy"
        );
    let mut context = serde_json::Map::new();
    context.insert("retryable".to_string(), Value::Bool(failure.retryable));
    if let Some(receipt) = failure.receipt.as_ref() {
        if let Ok(receipt) = serde_json::to_value(receipt) {
            context.insert("calendarReceipt".to_string(), receipt);
        }
    }
    if let Some(requested) = failure.requested_calendar_name {
        context.insert(
            "requestedCalendarName".to_string(),
            Value::String(requested),
        );
    }
    if !failure.available_calendar_names.is_empty() {
        context.insert(
            "availableCalendarNames".to_string(),
            Value::Array(
                failure
                    .available_calendar_names
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if verified_unchanged {
        context.insert("changedState".to_string(), Value::Bool(false));
    }
    json!({
        "taskToolError": {
            "code": failure.code,
            "message": failure.message,
            "context": context,
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_failure_keeps_bounded_choices_and_verified_unchanged_state() {
        for code in [
            "calendar_not_found",
            "calendar_name_ambiguous",
            "calendar_read_only",
            "calendar_availability_unsupported",
        ] {
            let raw = calendar_failure_error(CalendarReadFailure {
                code: code.to_string(),
                message: "The exact requested calendar needs a new target.".to_string(),
                retryable: false,
                requested_calendar_name: Some("OOMU Test".to_string()),
                available_calendar_names: vec!["Personal".to_string(), "Work".to_string()],
                receipt: None,
            });
            let decoded = crate::tools::task_tool_error::decode(
                &raw,
                "calendar_event_failed",
                "Calendar",
                "Calendar could not finish safely.",
            );
            assert_eq!(decoded.code, code);
            assert_eq!(decoded.context["requestedCalendarName"], "OOMU Test");
            assert_eq!(decoded.context["availableCalendarNames"][0], "Personal");
            assert_eq!(
                decoded.changed_state,
                crate::tools::task_tool_error::ChangedState::None,
                "{code}"
            );
            assert!(decoded.changed_state_verified, "{code}");
        }
    }

    #[test]
    fn every_pre_mutation_authorization_failure_is_verified_unchanged() {
        for code in [
            "calendar_permission_denied",
            "calendar_permission_restricted",
            "calendar_permission_write_only",
            "calendar_permission_unavailable",
            "calendar_authorization_timeout",
            "calendar_access_check_timeout",
            "calendar_store_reset_timeout",
            "calendar_source_refresh_timeout",
        ] {
            let raw = calendar_failure_error(CalendarReadFailure {
                code: code.to_string(),
                message: "Calendar Full Access is required.".to_string(),
                retryable: code == "calendar_authorization_timeout",
                requested_calendar_name: None,
                available_calendar_names: Vec::new(),
                receipt: None,
            });
            let decoded = crate::tools::task_tool_error::decode(
                &raw,
                "calendar_event_failed",
                "Calendar",
                "Calendar could not finish safely.",
            );
            assert_eq!(decoded.code, code);
            assert_eq!(
                decoded.changed_state,
                crate::tools::task_tool_error::ChangedState::None,
                "{code}"
            );
            assert!(decoded.changed_state_verified, "{code}");
        }
    }
}
