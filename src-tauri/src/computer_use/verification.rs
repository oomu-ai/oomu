use super::{
    contracts::{
        DesktopObservation, DesktopSemanticAction, ExpectedOutcomeKind, FileHashEvidence,
        ObservedPostcondition, MAX_ACTION_TEXT_CHARS,
    },
    driver::ResolvedDriverTarget,
    error::{AppControlError, AppControlErrorCode, AppControlResult},
    policy::ScopedFileRoots,
    state::{invalid_request, stale_reference},
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub(super) fn normalize_action(
    action: DesktopSemanticAction,
    roots: &ScopedFileRoots,
) -> AppControlResult<DesktopSemanticAction> {
    let valid_ref =
        |value: &str| value.starts_with("appref_") && value.len() <= 128 && value.is_ascii();
    if action.references().iter().any(|value| !valid_ref(value)) {
        return Err(stale_reference());
    }
    match action {
        DesktopSemanticAction::Select { reference, value } => {
            if value.is_empty() || value.chars().count() > MAX_ACTION_TEXT_CHARS {
                return Err(invalid_request("The selected value is invalid."));
            }
            Ok(DesktopSemanticAction::Select { reference, value })
        }
        DesktopSemanticAction::TypeText { reference, text } => {
            if text.chars().count() > MAX_ACTION_TEXT_CHARS {
                return Err(invalid_request("The text is too long for one app action."));
            }
            Ok(DesktopSemanticAction::TypeText { reference, text })
        }
        DesktopSemanticAction::Scroll { reference, amount } => {
            if amount == 0 || amount.unsigned_abs() > 4_000 {
                return Err(invalid_request(
                    "The scroll amount is outside the safe bound.",
                ));
            }
            Ok(DesktopSemanticAction::Scroll { reference, amount })
        }
        DesktopSemanticAction::ChooseFile {
            reference,
            file_grant_id,
        } => {
            roots.canonical_granted_file(&file_grant_id)?;
            Ok(DesktopSemanticAction::ChooseFile {
                reference,
                file_grant_id,
            })
        }
        other => Ok(other),
    }
}

pub(super) fn resolved_file_for_action(
    action: &DesktopSemanticAction,
    roots: &ScopedFileRoots,
) -> AppControlResult<Option<PathBuf>> {
    match action {
        DesktopSemanticAction::ChooseFile { file_grant_id, .. } => {
            roots.canonical_granted_file(file_grant_id).map(Some)
        }
        _ => Ok(None),
    }
}

pub(super) fn hash_action_binding(
    action: &DesktopSemanticAction,
    selected_file: Option<&Path>,
) -> AppControlResult<String> {
    hash_serializable(&serde_json::json!({
        "action": action,
        "selectedFile": selected_file,
    }))
}

pub(super) fn verify_postcondition(
    postcondition: &ObservedPostcondition,
    expected: ExpectedOutcomeKind,
    action: &DesktopSemanticAction,
    targets: &[ResolvedDriverTarget],
    before: &DesktopObservation,
    after: &DesktopObservation,
    roots: &ScopedFileRoots,
) -> AppControlResult<Vec<FileHashEvidence>> {
    if postcondition.kind() != expected {
        return Err(postcondition_mismatch());
    }
    match postcondition {
        ObservedPostcondition::NoChange => {
            if semantic_observation_hash(before)? != semantic_observation_hash(after)? {
                return Err(postcondition_mismatch());
            }
            Ok(Vec::new())
        }
        ObservedPostcondition::ElementValue {
            element_key,
            value_sha256,
        } => {
            require_target_key(targets, element_key)?;
            let observed = after
                .elements
                .iter()
                .find(|element| element.element_key == *element_key)
                .and_then(|element| element.value_digest.as_deref());
            if !valid_sha256(value_sha256) || observed != Some(value_sha256) {
                return Err(postcondition_mismatch());
            }
            Ok(Vec::new())
        }
        ObservedPostcondition::ElementState { element_key, state } => {
            require_target_key(targets, element_key)?;
            let before_element = before
                .elements
                .iter()
                .find(|element| element.element_key == *element_key);
            let after_element = after
                .elements
                .iter()
                .find(|element| element.element_key == *element_key)
                .ok_or_else(postcondition_mismatch)?;
            let verified = match state.as_str() {
                "focused" => {
                    after.focused_element.as_deref() == Some(after_element.reference.as_str())
                }
                "value_changed" => {
                    before_element.and_then(|element| element.value_digest.as_ref())
                        != after_element.value_digest.as_ref()
                }
                _ => false,
            };
            if !verified {
                return Err(postcondition_mismatch());
            }
            Ok(Vec::new())
        }
        ObservedPostcondition::WindowState { window_id, state } => {
            let valid_state = match action {
                DesktopSemanticAction::ChooseFile { .. } => {
                    state == "file_selection_completed"
                        && before.window.modal
                        && !after.window.modal
                }
                _ => state == "focused_window_changed",
            };
            if window_id != &after.window.window_id
                || !valid_state
                || before.window.window_id == after.window.window_id
            {
                return Err(postcondition_mismatch());
            }
            Ok(Vec::new())
        }
        ObservedPostcondition::FileHash {
            canonical_path,
            sha256,
        } => {
            let path = roots.canonical_file(canonical_path)?;
            let actual = hash_file(&path)?;
            if !valid_sha256(sha256) || actual != *sha256 {
                return Err(postcondition_mismatch());
            }
            Ok(vec![FileHashEvidence {
                canonical_path: path,
                sha256: actual,
            }])
        }
        ObservedPostcondition::ApplicationState { state } => {
            let verified = match action {
                DesktopSemanticAction::DragDrop { .. } => {
                    state == "finder_items_changed"
                        && before.application.bundle_id == "com.apple.finder"
                        && semantic_observation_hash(before)? != semantic_observation_hash(after)?
                }
                DesktopSemanticAction::AppleEvent { .. } => {
                    state == "application_activated"
                        && before.application.bundle_id == after.application.bundle_id
                        && before.application.process_id == after.application.process_id
                        && after.window.visible
                }
                _ => {
                    state == "accessibility_state_changed"
                        && semantic_observation_hash(before)? != semantic_observation_hash(after)?
                }
            };
            if !verified {
                return Err(postcondition_mismatch());
            }
            Ok(Vec::new())
        }
    }
}

fn semantic_observation_hash(observation: &DesktopObservation) -> AppControlResult<String> {
    let focused_key = observation.focused_element.as_ref().and_then(|reference| {
        observation
            .elements
            .iter()
            .find(|element| &element.reference == reference)
            .map(|element| element.element_key.as_str())
    });
    let elements = observation
        .elements
        .iter()
        .map(|element| {
            serde_json::json!({
                "key": element.element_key,
                "role": element.role,
                "label": element.label,
                "valueDigest": element.value_digest,
                "secure": element.secure,
                "visible": element.visible,
                "enabled": element.enabled,
                "inModal": element.in_modal,
                "supportedActions": element.supported_actions,
                "geometry": element.geometry,
            })
        })
        .collect::<Vec<_>>();
    hash_serializable(&serde_json::json!({
        "application": observation.application,
        "window": observation.window,
        "focusedKey": focused_key,
        "elements": elements,
        "screenshot": observation.screenshot,
    }))
}

fn require_target_key(targets: &[ResolvedDriverTarget], key: &str) -> AppControlResult<()> {
    if targets.iter().any(|target| target.element_key == key) {
        Ok(())
    } else {
        Err(postcondition_mismatch())
    }
}

pub(super) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn hash_serializable(value: &impl serde::Serialize) -> AppControlResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| hash_bytes(&bytes))
        .map_err(|_| {
            AppControlError::new(
                AppControlErrorCode::DriverFailure,
                "App control could not hash its verification evidence.",
            )
        })
}

pub(super) fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn hash_file(path: &Path) -> AppControlResult<String> {
    let mut file = File::open(path).map_err(|_| postcondition_mismatch())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| postcondition_mismatch())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn postcondition_mismatch() -> AppControlError {
    AppControlError::new(
        AppControlErrorCode::PostconditionMismatch,
        "The app did not reach the verified result required by this action.",
    )
}
