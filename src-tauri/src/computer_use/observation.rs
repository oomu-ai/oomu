use super::{
    contracts::{
        AccessibilityPermission, AppControlPauseReason, AppControlState, DesktopObservation,
        MAX_OBSERVED_ELEMENTS, REFERENCE_TTL_MS,
    },
    driver::{DriverActionRequest, DriverObservation, DriverObservationRequest},
    error::{AppControlError, AppControlErrorCode, AppControlResult},
    manager::{ManagerState, SessionRecord},
    policy::{app_profile, ApplicationQualification},
    references::{opaque_id, ReferenceContext},
    state::{invalid_request, pause_session, session_not_found, valid_bundle_id},
    verification::{hash_serializable, valid_sha256},
};
use std::collections::HashSet;

pub(super) fn apply_observation(
    state: &mut ManagerState,
    session_id: &str,
    raw: DriverObservation,
    now: i64,
    allow_window_transition: bool,
) -> AppControlResult<DesktopObservation> {
    validate_raw_observation(&raw)?;
    let ManagerState {
        sessions,
        references,
        ..
    } = state;
    let session = sessions.get_mut(session_id).ok_or_else(session_not_found)?;
    if raw.permission != AccessibilityPermission::Granted {
        pause_session(
            session,
            references,
            now,
            AppControlPauseReason::PermissionChanged,
        );
        return Err(AppControlError::new(
            AppControlErrorCode::AccessibilityPermissionMissing,
            "Accessibility permission is required for app control.",
        ));
    }

    let profile = app_profile(&raw.application.bundle_id, &raw.application.display_name);
    if profile.qualification == ApplicationQualification::Browser {
        session.state = AppControlState::Failed;
        session.current_action = None;
        session.updated_at_ms = now;
        references.invalidate_session(session_id);
        return Err(AppControlError::new(
            AppControlErrorCode::BrowserRouteRequired,
            "Browser work must use the guarded browser runtime.",
        ));
    }
    if profile.qualification == ApplicationQualification::Qualified
        && !session
            .approved_bundle_ids
            .contains(&raw.application.bundle_id)
    {
        return Err(AppControlError::new(
            AppControlErrorCode::ApplicationNotAllowlisted,
            "This application is not approved for the current Task.",
        ));
    }

    let is_first_observation = session.last_observation.is_none();
    let application_changed = session
        .last_observation
        .as_ref()
        .is_some_and(|observation| {
            observation.application.bundle_id != raw.application.bundle_id
                || observation.application.process_id != raw.application.process_id
        });
    let window_changed = session
        .last_observation
        .as_ref()
        .is_some_and(|observation| observation.window.window_id != raw.window.window_id);
    if application_changed {
        pause_session(
            session,
            references,
            now,
            AppControlPauseReason::ApplicationChanged,
        );
    } else if window_changed && !allow_window_transition {
        pause_session(
            session,
            references,
            now,
            AppControlPauseReason::UnexpectedNavigation,
        );
    } else {
        references.invalidate_session(session_id);
    }

    session.revision = session.revision.saturating_add(1);
    let revision = session.revision;
    let generation = session.generation;
    let expires_at_ms = now.saturating_add(REFERENCE_TTL_MS);
    let context = ReferenceContext {
        session_id: &session.session_id,
        project_id: &session.project_id,
        task_run_id: &session.task_run_id,
        bundle_id: &raw.application.bundle_id,
        process_id: raw.application.process_id,
        window_id: &raw.window.window_id,
        revision,
        generation,
        now_ms: now,
    };
    let focused_key = raw.focused_element_key.clone();
    let mut focused_element = None;
    let mut focused_secure = false;
    let mut elements = Vec::with_capacity(raw.elements.len());
    for element in raw.elements {
        let key_matches = focused_key.as_deref() == Some(element.element_key.as_str());
        focused_secure |= key_matches && element.secure;
        let observed = references.issue(&context, element, expires_at_ms);
        if key_matches {
            focused_element = Some(observed.reference.clone());
        }
        elements.push(observed);
    }

    let mut observation = DesktopObservation {
        observation_id: opaque_id("appobservation"),
        session_id: session.session_id.clone(),
        project_id: session.project_id.clone(),
        task_run_id: session.task_run_id.clone(),
        revision,
        generation,
        observed_at_ms: now,
        expires_at_ms,
        permission: raw.permission,
        application: raw.application,
        window: raw.window,
        focused_element,
        elements,
        screenshot: raw.screenshot,
        observation_hash: String::new(),
    };
    observation.observation_hash = hash_serializable(&observation)?;

    session.last_observation = Some(observation.clone());
    session.updated_at_ms = now;
    if !application_changed && (!window_changed || allow_window_transition) {
        if session.state == AppControlState::ReturnPending
            || (is_first_observation && session.state == AppControlState::Observing)
        {
            session.state = if profile.qualification == ApplicationQualification::Qualified {
                AppControlState::Running
            } else {
                AppControlState::Observing
            };
            session.pause_reason = None;
        }
    }
    if !observation.window.visible {
        pause_session(
            session,
            references,
            now,
            AppControlPauseReason::HiddenWindow,
        );
    } else if focused_secure {
        pause_session(session, references, now, AppControlPauseReason::SecureField);
    }
    Ok(observation)
}

fn validate_raw_observation(raw: &DriverObservation) -> AppControlResult<()> {
    if raw.application.process_id == 0
        || !valid_bundle_id(&raw.application.bundle_id)
        || raw.application.display_name.trim().is_empty()
        || raw.application.display_name.chars().count() > 120
        || raw.window.window_id.trim().is_empty()
        || raw.window.window_id.chars().count() > 256
        || raw.elements.len() > MAX_OBSERVED_ELEMENTS
    {
        return Err(invalid_request("The native app observation is malformed."));
    }
    if let Some(screenshot) = &raw.screenshot {
        if !valid_sha256(&screenshot.sha256) || screenshot.width == 0 || screenshot.height == 0 {
            return Err(invalid_request("The screenshot receipt is malformed."));
        }
    }
    let mut keys = HashSet::with_capacity(raw.elements.len());
    for element in &raw.elements {
        if element.element_key.trim().is_empty()
            || element.element_key.chars().count() > 512
            || element.role.trim().is_empty()
            || element.role.chars().count() > 120
            || !keys.insert(element.element_key.as_str())
        {
            return Err(invalid_request(
                "The accessibility element set is malformed.",
            ));
        }
        if element.secure && element.value_digest.is_some() {
            return Err(invalid_request(
                "Protected accessibility values must be redacted by the native driver.",
            ));
        }
    }
    if raw
        .focused_element_key
        .as_ref()
        .is_some_and(|focused| !keys.contains(focused.as_str()))
    {
        return Err(invalid_request(
            "The focused element is not in the observation.",
        ));
    }
    Ok(())
}

pub(super) fn reference_context<'a>(
    session: &'a SessionRecord,
    observation: &'a DesktopObservation,
    now_ms: i64,
) -> ReferenceContext<'a> {
    ReferenceContext {
        session_id: &session.session_id,
        project_id: &session.project_id,
        task_run_id: &session.task_run_id,
        bundle_id: &observation.application.bundle_id,
        process_id: observation.application.process_id,
        window_id: &observation.window.window_id,
        revision: observation.revision,
        generation: observation.generation,
        now_ms,
    }
}

pub(super) fn driver_observation_request(session: &SessionRecord) -> DriverObservationRequest {
    DriverObservationRequest {
        session_id: session.session_id.clone(),
        project_id: session.project_id.clone(),
        task_run_id: session.task_run_id.clone(),
    }
}

pub(super) fn driver_observation_request_from(
    request: &DriverActionRequest,
) -> DriverObservationRequest {
    DriverObservationRequest {
        session_id: request.session_id.clone(),
        project_id: request.project_id.clone(),
        task_run_id: request.task_run_id.clone(),
    }
}
