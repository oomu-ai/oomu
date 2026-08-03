use super::{
    contracts::{
        AppControlApplicationView, AppControlPauseReason, AppControlSessionView, AppControlState,
    },
    error::{AppControlError, AppControlErrorCode, AppControlResult},
    manager::{ManagerState, SessionRecord},
    policy::app_profile,
    references::ReferenceVault,
};
use std::sync::atomic::Ordering;

pub(super) fn session_view(session: &SessionRecord) -> AppControlSessionView {
    let application = session.last_observation.as_ref().map(|observation| {
        let profile = app_profile(
            &observation.application.bundle_id,
            &observation.application.display_name,
        );
        AppControlApplicationView {
            name: profile.display_name,
            icon: profile.icon,
        }
    });
    AppControlSessionView {
        session_id: session.session_id.clone(),
        task_run_id: session.task_run_id.clone(),
        project_id: session.project_id.clone(),
        state: session.state,
        application,
        current_action: session.current_action.clone(),
        pause_reason: session.pause_reason,
        can_pause: matches!(
            session.state,
            AppControlState::Observing | AppControlState::Running | AppControlState::ReturnPending
        ),
        can_take_control: matches!(
            session.state,
            AppControlState::Observing | AppControlState::Running | AppControlState::Paused
        ),
        can_return_to_oomu: matches!(
            session.state,
            AppControlState::Takeover | AppControlState::Paused
        ),
        observation_generation: session.generation,
        last_outcome: session.last_outcome.clone(),
        updated_at_ms: session.updated_at_ms,
    }
}

pub(super) fn pause_session(
    session: &mut SessionRecord,
    references: &mut ReferenceVault,
    now: i64,
    reason: AppControlPauseReason,
) {
    invalidate_generation(session, references, now);
    session.state = AppControlState::Paused;
    session.pause_reason = Some(reason);
}

pub(super) fn invalidate_generation(
    session: &mut SessionRecord,
    references: &mut ReferenceVault,
    now: i64,
) {
    session.generation = session.generation.saturating_add(1);
    session.cancellation_epoch.fetch_add(1, Ordering::SeqCst);
    session.current_action = None;
    session.updated_at_ms = now;
    references.invalidate_session(&session.session_id);
}

pub(super) fn ensure_generation(
    state: &ManagerState,
    session_id: &str,
    task_run_id: &str,
    generation: u64,
    revision: Option<u64>,
) -> AppControlResult<()> {
    let session = state
        .sessions
        .get(session_id)
        .ok_or_else(session_not_found)?;
    require_task(session, task_run_id)?;
    if session.generation != generation || revision.is_some_and(|value| session.revision != value) {
        return Err(stale_reference());
    }
    Ok(())
}

pub(super) fn require_task(session: &SessionRecord, task_run_id: &str) -> AppControlResult<()> {
    if session.task_run_id == task_run_id {
        Ok(())
    } else {
        Err(AppControlError::new(
            AppControlErrorCode::TaskBindingMismatch,
            "This app control session belongs to a different Task.",
        ))
    }
}

pub(super) fn valid_bundle_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.is_ascii()
        && value.split('.').count() >= 2
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

pub(super) fn invalid_request(message: impl Into<String>) -> AppControlError {
    AppControlError::new(AppControlErrorCode::InvalidRequest, message)
}

pub(super) fn session_not_found() -> AppControlError {
    AppControlError::new(
        AppControlErrorCode::SessionNotFound,
        "The app control session was not found.",
    )
}

pub(super) fn not_running() -> AppControlError {
    AppControlError::new(
        AppControlErrorCode::NotRunning,
        "This app control session is not ready to run an action.",
    )
}

pub(super) fn stale_reference() -> AppControlError {
    AppControlError::new(
        AppControlErrorCode::StaleReference,
        "The screen changed; a fresh observation is required.",
    )
}
