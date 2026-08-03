use super::{
    contracts::{DesktopSemanticAction, ExpectedOutcomeKind, ObservedPostcondition},
    driver::DriverActionRequest,
    error::{AppControlError, AppControlErrorCode, AppControlResult},
};
use sha2::{Digest, Sha256};

pub(super) fn action_postcondition(
    request: &DriverActionRequest,
    after_window_id: Option<String>,
) -> AppControlResult<ObservedPostcondition> {
    let target_key = request
        .targets
        .first()
        .map(|target| target.element_key.clone());
    match request.expected_outcome {
        ExpectedOutcomeKind::NoChange => Ok(ObservedPostcondition::NoChange),
        ExpectedOutcomeKind::ElementValue => {
            let value = match &request.action {
                DesktopSemanticAction::Select { value, .. } => value,
                DesktopSemanticAction::TypeText { text, .. } => text,
                _ => return Err(driver_error("This action cannot prove an element value.")),
            };
            Ok(ObservedPostcondition::ElementValue {
                element_key: target_key.ok_or_else(stale_action)?,
                value_sha256: hex::encode(Sha256::digest(value.as_bytes())),
            })
        }
        ExpectedOutcomeKind::ElementState => Ok(ObservedPostcondition::ElementState {
            element_key: target_key.ok_or_else(stale_action)?,
            state: match &request.action {
                DesktopSemanticAction::Focus { .. } => "focused",
                DesktopSemanticAction::Press { .. } | DesktopSemanticAction::Select { .. } => {
                    "value_changed"
                }
                _ => {
                    return Err(driver_error(
                        "This action cannot prove the requested element state.",
                    ))
                }
            }
            .to_string(),
        }),
        ExpectedOutcomeKind::WindowState => Ok(ObservedPostcondition::WindowState {
            window_id: after_window_id.ok_or_else(|| {
                driver_error("The changed application window could not be verified.")
            })?,
            state: if matches!(&request.action, DesktopSemanticAction::ChooseFile { .. }) {
                "file_selection_completed"
            } else {
                "focused_window_changed"
            }
            .to_string(),
        }),
        ExpectedOutcomeKind::ApplicationState => Ok(ObservedPostcondition::ApplicationState {
            state: match &request.action {
                DesktopSemanticAction::DragDrop { .. } => "finder_items_changed",
                DesktopSemanticAction::AppleEvent { .. } => "application_activated",
                _ => "accessibility_state_changed",
            }
            .to_string(),
        }),
        ExpectedOutcomeKind::FileHash => Err(driver_error(
            "File verification needs a reviewed app-specific adapter.",
        )),
    }
}

fn driver_error(message: impl Into<String>) -> AppControlError {
    AppControlError::new(AppControlErrorCode::DriverFailure, message)
}

fn stale_action() -> AppControlError {
    AppControlError::new(
        AppControlErrorCode::StaleReference,
        "The screen changed before the app action could commit.",
    )
}
