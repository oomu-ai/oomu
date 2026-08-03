use super::*;

const CALENDAR_ACTIONS: [&str; 3] = [
    "create_conflict_free_calendar_event",
    "create_system_calendar_event",
    "create_release_recovery_calendar_event",
];

pub(super) fn require_prior_step_receipts(
    step_index: usize,
    completed_receipt_count: usize,
) -> Result<(), AgenticLoopError> {
    if completed_receipt_count == step_index {
        return Ok(());
    }
    Err(AgenticLoopError {
        code: "execution_checkpoint_sequence_invalid",
        boundary: "PersistentStateEngine",
        message: "OOMU could not safely continue because the next permission request was not bound to every earlier completed-step receipt. Nothing was replayed."
            .to_string(),
        mlc_path: None,
    })
}

pub(super) async fn preflight_action_permission(
    action: &RequestedAction,
    app: Option<&tauri::AppHandle>,
    persistence: &PersistenceEngine,
    session_id: Option<&str>,
    agent_id: Option<&str>,
    origin_guard: Option<&AgentExecutionOriginGuard>,
) -> Result<(), AgenticLoopError> {
    if let Some(origin_guard) = origin_guard {
        origin_guard.ensure_current()?;
    }
    let Some(mut approval) = build_shield_approval_request(action) else {
        return Ok(());
    };
    let is_filesystem_scope = matches!(
        approval.action_class.as_str(),
        "filesystem_read" | "filesystem_write"
    );
    let normally_allowed = authorize_action(action.clone()).is_ok();
    let approved_shape_is_valid = authorize_action_for_approved_plan(action.clone()).is_ok();
    let external_filesystem_access =
        is_filesystem_scope && !normally_allowed && approved_shape_is_valid;
    let requires_one_time_reconfirmation = (approval.mandatory_reconfirm
        || crate::tools::task_tool_runtime::requires_explicit_approval(&action.kind))
        && approved_shape_is_valid;
    if !external_filesystem_access && !requires_one_time_reconfirmation {
        return Ok(());
    }
    let Some(app) = app else {
        return Err(AgenticLoopError {
            code: "permission_prompt_unavailable",
            boundary: "ShieldApprovalManager",
            message: "OOMU couldn’t ask for permission. Nothing was changed. Try again."
                .to_string(),
            mlc_path: None,
        });
    };
    let calendar_denial_context = if is_calendar_action(action) {
        let (requested_calendar_name, availability) = calendar_name_and_availability(action)
            .ok_or_else(|| AgenticLoopError {
                code: "calendar_recovery_context_invalid",
                boundary: "ShieldApprovalManager",
                message: "OOMU couldn’t bind this Calendar approval to its recovery target. Nothing was changed."
                    .to_string(),
                mlc_path: None,
            })?;
        let available_calendar_names =
            crate::tools::eventkit_calendar::compatible_calendar_names(availability)
                .await
                .map_err(|failure| calendar_recovery_targets_error(action, failure))?;
        Some((requested_calendar_name, available_calendar_names))
    } else {
        None
    };
    let principal = agent_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("local_principal");
    approval.session_id = session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    approval.principal = Some(principal.to_string());
    if let Some(origin_guard) = origin_guard {
        approval.turn_id = Some(origin_guard.context.turn_id.clone());
        approval.generation_token = Some(origin_guard.context.generation_token.clone());
    }

    if !requires_one_time_reconfirmation {
        let scope_trust = app.state::<ScopeTrustManager>();
        let session_authorized = scope_trust
            .allows_action_for_principal(&action, principal)
            .map_err(|error| AgenticLoopError {
                code: error.code,
                boundary: error.boundary,
                message: "OOMU couldn’t check this permission. Nothing was changed. Try again."
                    .to_string(),
                mlc_path: None,
            })?;
        let resource = approval
            .canonical_resource
            .as_deref()
            .or(approval.scope_trust_prefix.as_deref())
            .unwrap_or(approval.action_type.as_str());
        let mut durable_authorized = crate::approval_scopes::authorize(
            persistence,
            principal,
            approval.project_id.as_deref(),
            approval.task_run_id.as_deref(),
            &approval.action_class,
            resource,
            &approval.argument_class,
            1,
        )
        .map_err(|_| AgenticLoopError {
            code: "permission_check_failed",
            boundary: "ReviewedApprovalScope",
            message: "OOMU couldn’t check this permission. Nothing was changed. Try again."
                .to_string(),
            mlc_path: None,
        })?;
        if !durable_authorized && approval.action_class == "filesystem_write" {
            let legacy_action_class = action.kind.trim().replace('-', "_").to_ascii_lowercase();
            let legacy_argument_class =
                crate::approval_scopes::argument_class(&legacy_action_class, &approval.preview);
            durable_authorized = crate::approval_scopes::authorize(
                persistence,
                principal,
                approval.project_id.as_deref(),
                approval.task_run_id.as_deref(),
                &legacy_action_class,
                resource,
                &legacy_argument_class,
                1,
            )
            .map_err(|_| AgenticLoopError {
                code: "permission_check_failed",
                boundary: "ReviewedApprovalScope",
                message: "OOMU couldn’t check this permission. Nothing was changed. Try again."
                    .to_string(),
                mlc_path: None,
            })?;
        }
        if session_authorized || durable_authorized {
            return Ok(());
        }
    }

    let approvals = app.state::<ShieldApprovalManager>();
    if let Err(error) = request_user_approval(app, approvals.inner(), approval).await {
        if error.code == "shield_approval_denied" {
            if let Some((requested_calendar_name, available_calendar_names)) =
                calendar_denial_context
            {
                return Err(calendar_action_denied_error_with_context(
                    action,
                    requested_calendar_name,
                    available_calendar_names,
                ));
            }
        }
        return Err(AgenticLoopError {
            code: if error.code == "shield_approval_denied" {
                "permission_denied"
            } else {
                "permission_request_failed"
            },
            boundary: "ShieldApprovalManager",
            message: if error.code == "shield_approval_denied" {
                "Permission wasn’t granted. Nothing was changed."
            } else {
                "OOMU couldn’t ask for permission. Nothing was changed. Try again."
            }
            .to_string(),
            mlc_path: None,
        });
    }
    if let Some(origin_guard) = origin_guard {
        origin_guard.ensure_current()?;
    }

    Ok(())
}

fn is_calendar_action(action: &RequestedAction) -> bool {
    CALENDAR_ACTIONS.contains(
        &action
            .kind
            .trim()
            .replace('-', "_")
            .to_ascii_lowercase()
            .as_str(),
    )
}

fn calendar_action_arguments(action: &RequestedAction) -> Option<serde_json::Value> {
    action
        .content
        .as_deref()
        .and_then(|content| serde_json::from_str(content).ok())
}

fn calendar_name_and_availability(
    action: &RequestedAction,
) -> Option<(
    String,
    crate::tools::eventkit_calendar::CalendarEventAvailability,
)> {
    let arguments = calendar_action_arguments(action)?;
    let calendar_name = arguments.get("calendarName")?.as_str()?.trim().to_string();
    if calendar_name.is_empty() {
        return None;
    }
    let availability = arguments
        .get("availability")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or(crate::tools::eventkit_calendar::CalendarEventAvailability::Tentative);
    Some((calendar_name, availability))
}

fn calendar_recovery_targets_error(
    action: &RequestedAction,
    failure: crate::tools::eventkit_calendar::CalendarReadFailure,
) -> AgenticLoopError {
    let raw = crate::tools::system_calendar_event::calendar_failure_error(failure);
    AgenticLoopError {
        code: "calendar_recovery_targets_unavailable",
        boundary: "CalendarRecovery",
        message: crate::tools::task_tool_runtime::normalize_agent_error(&action.kind, &raw),
        mlc_path: None,
    }
}

fn calendar_action_denied_error_with_context(
    action: &RequestedAction,
    requested_calendar_name: String,
    available_calendar_names: Vec<String>,
) -> AgenticLoopError {
    let arguments_sha256 = action
        .content
        .as_deref()
        .map(|content| crate::foundation::digest::sha256_hex(content.as_bytes()))
        .unwrap_or_else(|| crate::foundation::digest::sha256_hex(b""));
    let raw = serde_json::json!({
        "taskToolError": {
            "code": "calendar_action_denied",
            "message": "The Calendar event was not created because you denied this action. Choose another calendar to continue from your saved progress.",
            "context": {
                "requestedCalendarName": requested_calendar_name,
                "availableCalendarNames": available_calendar_names,
                "calendarStepArgumentsSha256": arguments_sha256,
                "deniedActionOperation": action.kind,
                "changedState": false
            }
        }
    })
    .to_string();
    AgenticLoopError {
        code: "calendar_action_denied",
        boundary: "ShieldApprovalManager",
        message: crate::tools::task_tool_runtime::normalize_agent_error(&action.kind, &raw),
        mlc_path: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_step_preflight_requires_every_prior_checkpoint_receipt() {
        assert!(require_prior_step_receipts(0, 0).is_ok());
        assert!(require_prior_step_receipts(1, 1).is_ok());
        assert!(require_prior_step_receipts(2, 2).is_ok());

        let missing_first = require_prior_step_receipts(1, 0)
            .expect_err("Calendar preflight cannot run before the agenda receipt exists");
        assert_eq!(missing_first.code, "execution_checkpoint_sequence_invalid");
        assert_eq!(missing_first.boundary, "PersistentStateEngine");
        assert!(require_prior_step_receipts(2, 1).is_err());
    }

    #[test]
    fn release_recovery_calendar_approval_context_begins_at_event_step() {
        let agenda = RequestedAction {
            kind: "prepare_release_recovery_agenda".to_string(),
            principal: None,
            path: Some("/testing/ship_test_02/release_recovery_agenda.md".to_string()),
            content: Some("{}".to_string()),
        };
        let calendar = RequestedAction {
            kind: "create_release_recovery_calendar_event".to_string(),
            principal: None,
            path: None,
            content: Some("{}".to_string()),
        };

        assert!(!is_calendar_action(&agenda));
        assert!(is_calendar_action(&calendar));
    }

    #[test]
    fn calendar_denial_is_typed_unchanged_and_bound_to_the_frozen_arguments() {
        let action = RequestedAction {
            kind: "create_release_recovery_calendar_event".to_string(),
            principal: None,
            path: None,
            content: Some(
                serde_json::json!({
                    "calendarName": "Initial Test",
                    "title": "OOMU Release Readiness",
                    "startDate": "2026-07-21T13:30:00-04:00",
                    "endDate": "2026-07-21T14:00:00-04:00",
                    "availability": "tentative"
                })
                .to_string(),
            ),
        };
        let error = calendar_action_denied_error_with_context(
            &action,
            "Initial Test".to_string(),
            vec!["OOMU Test".to_string()],
        );
        let parsed = crate::tools::task_tool_runtime::parse_agent_error(&error.message)
            .expect("typed Calendar denial");
        assert_eq!(parsed.code, "calendar_action_denied");
        assert_eq!(
            parsed.changed_state,
            crate::tools::task_tool_runtime::TaskToolChangedState::None
        );
        assert!(parsed.changed_state_verified);
        assert_eq!(parsed.context["requestedCalendarName"], "Initial Test");
        assert_eq!(parsed.context["availableCalendarNames"][0], "OOMU Test");
        assert_eq!(
            parsed.context["deniedActionOperation"],
            "create_release_recovery_calendar_event"
        );
        assert_eq!(
            parsed.context["calendarStepArgumentsSha256"],
            crate::foundation::digest::sha256_hex(action.content.unwrap().as_bytes())
        );
    }
}
