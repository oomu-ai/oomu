use super::{AgenticLoopError, RequestedAction};
use crate::tools::eventkit_calendar::CalendarReadFailure;

pub(super) fn action_requires_calendar_full_access(action: &RequestedAction) -> bool {
    matches!(
        action
            .kind
            .trim()
            .replace('-', "_")
            .to_ascii_lowercase()
            .as_str(),
        "create_conflict_free_calendar_event"
            | "create_system_calendar_event"
            | "create_release_recovery_calendar_event"
            | "prepare_release_recovery_agenda"
    )
}

pub(super) async fn preflight_calendar_full_access(
    action: &RequestedAction,
) -> Result<(), AgenticLoopError> {
    if !action_requires_calendar_full_access(action) {
        return Ok(());
    }
    crate::tools::eventkit_calendar::ensure_full_calendar_access()
        .await
        .map_err(|failure| calendar_permission_error(&action.kind, failure))
}

fn calendar_permission_error(operation: &str, failure: CalendarReadFailure) -> AgenticLoopError {
    let code = stable_calendar_permission_code(&failure.code);
    let raw = crate::tools::system_calendar_event::calendar_failure_error(failure);
    AgenticLoopError {
        code,
        boundary: "CalendarFullAccessPreflight",
        message: crate::tools::task_tool_runtime::normalize_agent_error(operation, &raw),
        mlc_path: None,
    }
}

fn stable_calendar_permission_code(code: &str) -> &'static str {
    match code {
        "calendar_permission_denied" => "calendar_permission_denied",
        "calendar_permission_restricted" => "calendar_permission_restricted",
        "calendar_permission_write_only" => "calendar_permission_write_only",
        "calendar_authorization_timeout" => "calendar_authorization_timeout",
        "calendar_authorization_failed" => "calendar_authorization_failed",
        "calendar_authorization_interrupted" => "calendar_authorization_interrupted",
        "calendar_access_check_timeout" => "calendar_access_check_timeout",
        "calendar_store_reset_timeout" => "calendar_store_reset_timeout",
        "calendar_source_refresh_timeout" => "calendar_source_refresh_timeout",
        "calendar_operation_busy" => "calendar_operation_busy",
        _ => "calendar_permission_unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_action(operation: &str) -> RequestedAction {
        RequestedAction {
            kind: operation.to_string(),
            principal: None,
            path: None,
            content: Some("{}".to_string()),
        }
    }

    #[test]
    fn only_the_current_calendar_action_triggers_the_preflight() {
        for operation in [
            "create_conflict_free_calendar_event",
            "create_system_calendar_event",
            "create_release_recovery_calendar_event",
            "prepare_release_recovery_agenda",
        ] {
            assert!(action_requires_calendar_full_access(&fixture_action(
                operation
            )));
        }
        assert!(!action_requires_calendar_full_access(&fixture_action(
            "draft_system_email"
        )));
    }

    #[test]
    fn later_calendar_step_does_not_trigger_during_current_file_write() {
        let current = fixture_action("create_file");
        let later = fixture_action("create_release_recovery_calendar_event");

        assert!(!action_requires_calendar_full_access(&current));
        assert!(action_requires_calendar_full_access(&later));
    }

    #[test]
    fn permission_preflight_errors_are_normalized_and_verified_unchanged() {
        for code in [
            "calendar_permission_denied",
            "calendar_permission_restricted",
            "calendar_permission_write_only",
            "calendar_permission_unavailable",
            "calendar_authorization_timeout",
            "calendar_authorization_failed",
            "calendar_authorization_interrupted",
            "calendar_access_check_timeout",
            "calendar_store_reset_timeout",
            "calendar_source_refresh_timeout",
            "calendar_operation_busy",
        ] {
            let error = calendar_permission_error(
                "create_conflict_free_calendar_event",
                CalendarReadFailure {
                    code: code.to_string(),
                    message: "Calendar Full Access is required.".to_string(),
                    retryable: code == "calendar_authorization_timeout",
                    requested_calendar_name: None,
                    available_calendar_names: Vec::new(),
                    receipt: None,
                },
            );
            let parsed = crate::tools::task_tool_runtime::parse_agent_error(&error.message)
                .expect("typed Calendar permission error");
            assert_eq!(parsed.code, code);
            assert_eq!(
                parsed.changed_state,
                crate::tools::task_tool_runtime::TaskToolChangedState::None,
                "{code}"
            );
            assert!(parsed.changed_state_verified, "{code}");
            assert_eq!(error.code, stable_calendar_permission_code(code), "{code}");
        }
    }
}
