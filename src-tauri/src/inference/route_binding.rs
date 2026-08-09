use super::{settings, InferenceError, DYNAMIC_ROUTE_ID};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DynamicRoutingMode {
    pub(super) active: bool,
    pub(super) preserve_session_binding: bool,
}

pub(super) fn resolve_dynamic_routing_mode_for_request(
    session_has_dynamic_binding: bool,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    has_explicit_turn_choice: bool,
    dynamic_routing_override: Option<bool>,
) -> DynamicRoutingMode {
    resolve_dynamic_routing_mode(
        session_has_dynamic_binding,
        is_dynamic_route_binding(provider_id, model_id),
        is_static_route_binding(provider_id, model_id),
        has_explicit_turn_choice,
        dynamic_routing_override,
    )
}

pub(super) fn initial_local_directory(
    app: &tauri::AppHandle,
    dynamic_routing_active: bool,
) -> Result<Option<PathBuf>, InferenceError> {
    dynamic_routing_active
        .then(|| settings::resolved_local_model_directory(app).map_err(InferenceError::worker))
        .transpose()
}

pub(super) fn auto_route_directory(directory: &Option<PathBuf>) -> &PathBuf {
    directory
        .as_ref()
        .expect("active Auto-route resolves its local model directory")
}

pub(super) fn final_local_directory(
    app: &tauri::AppHandle,
    directory: Option<PathBuf>,
    selected_route_is_local: bool,
) -> Result<PathBuf, InferenceError> {
    match directory {
        Some(directory) => Ok(directory),
        None if selected_route_is_local => {
            settings::resolved_local_model_directory(app).map_err(InferenceError::worker)
        }
        None => Ok(PathBuf::new()),
    }
}

pub(super) fn validate_turn_choice(
    has_choice: bool,
    dynamic_routing_active: bool,
    parent_turn_exists: bool,
) -> Result<(), InferenceError> {
    if has_choice && (!dynamic_routing_active || parent_turn_exists) {
        return Err(InferenceError::routing_attention(
            "auto_route_turn_choice_out_of_scope",
            "auto_route_turn_choice",
            "A per-turn Auto-route choice can only resume its original root Auto-route turn. Nothing was sent.",
        ));
    }
    Ok(())
}

pub(super) fn is_dynamic_route_binding(provider_id: Option<&str>, model_id: Option<&str>) -> bool {
    provider_id.is_some_and(is_dynamic_route_id) && model_id.is_some_and(is_dynamic_route_id)
}

pub(super) fn is_dynamic_route_id(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case(DYNAMIC_ROUTE_ID)
}

fn is_static_route_binding(provider_id: Option<&str>, model_id: Option<&str>) -> bool {
    provider_id.is_some_and(|value| !is_dynamic_route_id(value))
        && model_id.is_some_and(|value| !is_dynamic_route_id(value))
}

fn resolve_dynamic_routing_mode(
    session_has_dynamic_binding: bool,
    request_has_dynamic_binding: bool,
    request_has_static_binding: bool,
    has_explicit_turn_choice: bool,
    dynamic_routing_override: Option<bool>,
) -> DynamicRoutingMode {
    if request_has_static_binding && !has_explicit_turn_choice {
        return DynamicRoutingMode {
            active: false,
            preserve_session_binding: false,
        };
    }
    let implicit_dynamic_binding = session_has_dynamic_binding || request_has_dynamic_binding;
    let active = dynamic_routing_override.unwrap_or(implicit_dynamic_binding);
    DynamicRoutingMode {
        active,
        preserve_session_binding: active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_route_override_and_dynamic_binding_semantics_are_preserved() {
        assert!(is_dynamic_route_binding(Some("dynamic"), Some("dynamic")));
        assert!(!is_dynamic_route_binding(
            Some("dynamic"),
            Some("gemini-3.5-flash")
        ));

        let enabled =
            resolve_dynamic_routing_mode_for_request(false, None, None, false, Some(true));
        assert!(enabled.active);
        assert!(enabled.preserve_session_binding);

        let disabled = resolve_dynamic_routing_mode_for_request(
            true,
            Some("dynamic"),
            Some("dynamic"),
            false,
            Some(false),
        );
        assert!(!disabled.active);
        assert!(!disabled.preserve_session_binding);

        let implicit = resolve_dynamic_routing_mode_for_request(
            false,
            Some("dynamic"),
            Some("dynamic"),
            false,
            None,
        );
        assert!(implicit.active);
        assert!(implicit.preserve_session_binding);
    }

    #[test]
    fn explicit_cloud_binding_overrides_stale_auto_route_state() {
        let mode = resolve_dynamic_routing_mode_for_request(
            true,
            Some("gemini"),
            Some("gemini-3.5-flash"),
            false,
            Some(true),
        );
        assert!(!mode.active);
        assert!(!mode.preserve_session_binding);
    }

    #[test]
    fn explicit_auto_route_choice_preserves_the_frozen_auto_route_turn() {
        let mode = resolve_dynamic_routing_mode_for_request(
            true,
            Some("local-model"),
            Some("gemma-4-e2b"),
            true,
            Some(true),
        );
        assert!(mode.active);
        assert!(mode.preserve_session_binding);
    }
}
