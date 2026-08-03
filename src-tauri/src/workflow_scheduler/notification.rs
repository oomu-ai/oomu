use super::*;
use tauri_plugin_notification::{NotificationExt, PermissionState};

#[derive(Debug, Clone)]
pub(super) struct SchedulerCopy {
    pub(super) completed_title: String,
    completed_body: String,
    completed_empty_body: String,
    completed_declined_title: String,
    pub(super) completed_declined_body: String,
    pub(super) delivery_retry_title: String,
    pub(super) delivery_retry_body: String,
    pub(super) delivery_review_title: String,
    pub(super) delivery_review_body: String,
    approval_title: String,
    approval_body: String,
    pub(super) failed_title: String,
    failed_body: String,
    pub(super) run_failed_body: String,
    pub(super) delivery_completed: String,
    pub(super) delivery_completed_empty: String,
    pub(super) delivery_completed_verified: String,
    pub(super) delivery_completed_declined: String,
    pub(super) delivery_completed_declined_verified: String,
    pub(super) delivery_approval: String,
    pub(super) delivery_blocked: String,
    pub(super) delivery_failed: String,
    pub(super) delivery_failed_verified: String,
    pub(super) delivery_repair: String,
}

impl SchedulerCopy {
    pub(super) fn load(persistence: &PersistenceEngine) -> Self {
        settings::locale_state_for_engine(persistence, None)
            .map(|state| Self::from_translations(&state.translations))
            .unwrap_or_else(|_| Self::english())
    }

    pub(super) fn from_translations(translations: &Value) -> Self {
        let fallback = Self::english();
        let text = |pointer: &str, english: &str| {
            translations
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(english)
                .to_string()
        };
        Self {
            completed_title: text(
                "/workflow_scheduler/notification/completed_title",
                &fallback.completed_title,
            ),
            completed_body: text(
                "/workflow_scheduler/notification/completed_body",
                &fallback.completed_body,
            ),
            completed_empty_body: text(
                "/workflow_scheduler/notification/completed_empty_body",
                &fallback.completed_empty_body,
            ),
            completed_declined_title: text(
                "/workflow_scheduler/notification/completed_declined_title",
                &fallback.completed_declined_title,
            ),
            completed_declined_body: text(
                "/workflow_scheduler/notification/completed_declined_body",
                &fallback.completed_declined_body,
            ),
            delivery_retry_title: text(
                "/workflow_scheduler/notification/delivery_retry_title",
                &fallback.delivery_retry_title,
            ),
            delivery_retry_body: text(
                "/workflow_scheduler/notification/delivery_retry_body",
                &fallback.delivery_retry_body,
            ),
            delivery_review_title: text(
                "/workflow_scheduler/notification/delivery_review_title",
                &fallback.delivery_review_title,
            ),
            delivery_review_body: text(
                "/workflow_scheduler/notification/delivery_review_body",
                &fallback.delivery_review_body,
            ),
            approval_title: text(
                "/workflow_scheduler/notification/approval_title",
                &fallback.approval_title,
            ),
            approval_body: text(
                "/workflow_scheduler/notification/approval_body",
                &fallback.approval_body,
            ),
            failed_title: text(
                "/workflow_scheduler/notification/failed_title",
                &fallback.failed_title,
            ),
            failed_body: text(
                "/workflow_scheduler/notification/failed_body",
                &fallback.failed_body,
            ),
            run_failed_body: text(
                "/workflow_scheduler/notification/run_failed_body",
                &fallback.run_failed_body,
            ),
            delivery_completed: text(
                "/workflow_scheduler/delivery/completed",
                &fallback.delivery_completed,
            ),
            delivery_completed_empty: text(
                "/workflow_scheduler/delivery/completed_empty",
                &fallback.delivery_completed_empty,
            ),
            delivery_completed_verified: text(
                "/workflow_scheduler/delivery/completed_verified",
                &fallback.delivery_completed_verified,
            ),
            delivery_completed_declined: text(
                "/workflow_scheduler/delivery/completed_declined",
                &fallback.delivery_completed_declined,
            ),
            delivery_completed_declined_verified: text(
                "/workflow_scheduler/delivery/completed_declined_verified",
                &fallback.delivery_completed_declined_verified,
            ),
            delivery_approval: text(
                "/workflow_scheduler/delivery/approval",
                &fallback.delivery_approval,
            ),
            delivery_blocked: text(
                "/workflow_scheduler/delivery/blocked",
                &fallback.delivery_blocked,
            ),
            delivery_failed: text(
                "/workflow_scheduler/delivery/failed",
                &fallback.delivery_failed,
            ),
            delivery_failed_verified: text(
                "/workflow_scheduler/delivery/failed_verified",
                &fallback.delivery_failed_verified,
            ),
            delivery_repair: text(
                "/workflow_scheduler/delivery/repair",
                &fallback.delivery_repair,
            ),
        }
    }

    pub(super) fn english() -> Self {
        Self {
            completed_title: "Workflow Completed".to_string(),
            completed_body: "{name} completed successfully.".to_string(),
            completed_empty_body:
                "{name} completed. Nothing was found, so no later steps ran.".to_string(),
            completed_declined_title: "Workflow Finished".to_string(),
            completed_declined_body:
                "{name} finished. Not performed at your request: {actions}.".to_string(),
            delivery_retry_title: "Finishing Delivery".to_string(),
            delivery_retry_body:
                "{name}'s work is safe. OOMU is retrying the private-channel update without rerunning the workflow."
                    .to_string(),
            delivery_review_title: "Delivery Needs Attention".to_string(),
            delivery_review_body:
                "{name}'s work is safe, but OOMU could not confirm the private-channel update. Check the channel before retrying."
                    .to_string(),
            approval_title: "Workflow Needs Approval".to_string(),
            approval_body: "{name} is waiting for approval.".to_string(),
            failed_title: "Workflow Needs Attention".to_string(),
            failed_body:
                "{name} stopped before finishing{details} Open OOMU to review or retry."
                    .to_string(),
            run_failed_body:
                "{name} could not start: {error} Open OOMU to review or retry.".to_string(),
            delivery_completed:
                "{name} completed. Open OOMU Tasks for the verified result.".to_string(),
            delivery_completed_empty:
                "{name} completed. Nothing was found, so no later steps ran.".to_string(),
            delivery_completed_verified:
                "{name} completed successfully. Verified files: {filenames}.".to_string(),
            delivery_completed_declined:
                "{name} finished. Not performed at your request: {actions}. Open OOMU Tasks for the verified result."
                    .to_string(),
            delivery_completed_declined_verified:
                "{name} finished. Verified files: {filenames}. Not performed at your request: {actions}."
                    .to_string(),
            delivery_approval: "{name} needs approval. Reply '/approve {code} approve' or '/approve {code} deny' within 15 minutes.".to_string(),
            delivery_blocked:
                "{name} is blocked. Open OOMU Tasks to review the exact action.".to_string(),
            delivery_failed: "{name} failed. {error}".to_string(),
            delivery_failed_verified:
                "{fallback} Verified files kept: {filenames}.".to_string(),
            delivery_repair: "Open OOMU Tasks for repair details.".to_string(),
        }
    }
}

pub(super) fn render_scheduler_copy(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        output.push_str(&remaining[..start]);
        let after_open = &remaining[start + 1..];
        let Some(end) = after_open.find('}') else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let key = &after_open[..end];
        if let Some((_, value)) = replacements.iter().find(|(candidate, _)| *candidate == key) {
            output.push_str(value);
        } else {
            output.push_str(&remaining[start..start + end + 2]);
        }
        remaining = &after_open[end + 1..];
    }
    output.push_str(remaining);
    output
}

pub(super) fn notify_for_run_status(
    app: &tauri::AppHandle,
    schedule: &WorkflowScheduleRecord,
    status: ExecutionStatus,
    completion_kind: Option<WorkflowCompletionKind>,
    error_message: Option<&str>,
    copy: &SchedulerCopy,
    declined_actions: &[String],
) {
    if let Some((title, body)) = background_notice_copy(
        copy,
        &schedule_title(schedule),
        status,
        completion_kind,
        error_message,
        declined_actions,
    ) {
        notify_background_event(app, title, &body);
    }
}

pub(super) fn background_notice_copy<'a>(
    copy: &'a SchedulerCopy,
    schedule_title: &str,
    status: ExecutionStatus,
    completion_kind: Option<WorkflowCompletionKind>,
    error_message: Option<&str>,
    declined_actions: &[String],
) -> Option<(&'a str, String)> {
    match status {
        ExecutionStatus::Completed
            if completion_kind == Some(WorkflowCompletionKind::EmptyCollection) =>
        {
            Some((
                copy.completed_title.as_str(),
                render_scheduler_copy(&copy.completed_empty_body, &[("name", schedule_title)]),
            ))
        }
        ExecutionStatus::Completed if !declined_actions.is_empty() => {
            let actions = declined_actions.join(", ");
            Some((
                copy.completed_declined_title.as_str(),
                render_scheduler_copy(
                    &copy.completed_declined_body,
                    &[("name", schedule_title), ("actions", &actions)],
                ),
            ))
        }
        ExecutionStatus::Completed => Some((
            copy.completed_title.as_str(),
            render_scheduler_copy(&copy.completed_body, &[("name", schedule_title)]),
        )),
        ExecutionStatus::AwaitingApproval => Some((
            copy.approval_title.as_str(),
            render_scheduler_copy(&copy.approval_body, &[("name", schedule_title)]),
        )),
        ExecutionStatus::Failed => Some((
            copy.failed_title.as_str(),
            render_scheduler_copy(
                &copy.failed_body,
                &[
                    ("name", schedule_title),
                    (
                        "details",
                        &error_message
                            .map(|message| format!(": {message}"))
                            .unwrap_or_else(|| ".".to_string()),
                    ),
                ],
            ),
        )),
        ExecutionStatus::Pending | ExecutionStatus::Running => None,
    }
}

pub(super) fn notify_background_event(app: &tauri::AppHandle, title: &str, body: &str) {
    let notification = app.notification();
    let permission = match notification.permission_state() {
        Ok(PermissionState::Granted) => PermissionState::Granted,
        Ok(PermissionState::Denied) => {
            eprintln!("WORKFLOW_SCHEDULER_NOTIFICATION_DENIED title={title}");
            return;
        }
        Ok(PermissionState::Prompt | PermissionState::PromptWithRationale) => {
            match notification.request_permission() {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("WORKFLOW_SCHEDULER_NOTIFICATION_PERMISSION_FAILED {error}");
                    return;
                }
            }
        }
        Err(error) => {
            eprintln!("WORKFLOW_SCHEDULER_NOTIFICATION_STATE_FAILED {error}");
            return;
        }
    };
    if permission != PermissionState::Granted {
        eprintln!("WORKFLOW_SCHEDULER_NOTIFICATION_NOT_GRANTED state={permission}");
        return;
    }
    if let Err(error) = notification
        .builder()
        .title(title)
        .body(body)
        .group("oomu-workflow-schedules")
        .auto_cancel()
        .show()
    {
        eprintln!("WORKFLOW_SCHEDULER_NOTIFICATION_FAILED title={title} error={error}");
    }
}
