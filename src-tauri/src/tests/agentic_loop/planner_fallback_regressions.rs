use super::*;

fn planner_output_unusable_error(message: &str) -> AgenticLoopError {
    AgenticLoopError {
        code: "planner_output_unusable",
        boundary: "AgentPlanning",
        message: message.to_string(),
        mlc_path: None,
    }
}

#[test]
fn planner_output_unusable_selects_exact_e4b_fallback_target() {
    let planner_error = planner_output_unusable_error(
        "The planning response did not match OOMU's required action format after one repair attempt. Your request is saved and no action was executed. Try again or choose another model.",
    );

    let target = cloud_planner_local_fallback_target(&planner_error)
        .expect("schema exhaustion selects the bounded local fallback");
    match &target {
        PlannerExecutionTarget::Local {
            model_id: Some(model_id),
            reason,
        } => {
            assert_eq!(model_id, CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID);
            assert!(reason.contains("Gemma 4 E4B"));
        }
        _ => panic!("schema exhaustion must select the explicit E4B target"),
    }

    let route = ModelRouter::route(
        "Run local diagnostics.",
        ModelRoutePreference::GeminiPro,
        &diagnostics_draft(),
        0,
        &target,
    );
    assert_eq!(route.selected_model.name, "Gemma 4 E4B");
    assert_eq!(
        route.selected_model.version,
        CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID
    );
    assert_eq!(route.selected_model.provider, "Local");
    assert_eq!(route.selected_model.locality, "local");
    assert!(route.reason.contains("Gemma 4 E4B"));
}

#[test]
fn non_schema_cloud_failure_never_selects_local_fallback() {
    let planner_error = AgenticLoopError {
        code: "provider_network_error",
        boundary: "AgentPlanning",
        message: "Cloud planning could not connect. No action was executed.".to_string(),
        mlc_path: None,
    };

    assert!(cloud_planner_local_fallback_target(&planner_error).is_none());
}

#[test]
fn unusable_e4b_fallback_preserves_typed_cloud_exhaustion() {
    let safe_message = "The planning response did not match OOMU's required action format after one repair attempt. Your request is saved and no action was executed. Try again or choose another model.";
    let original_error = planner_output_unusable_error(safe_message);
    let fallback_target = cloud_planner_local_fallback_target(&original_error)
        .expect("schema exhaustion selects the local fallback");
    let unusable_draft = crate::gemma::generated_plan_from_text(
        "Run local diagnostics.".to_string(),
        "not an action plan".to_string(),
    );

    let error = match finalize_cloud_planner_local_fallback(
        "Run local diagnostics.",
        unusable_draft,
        original_error,
        fallback_target,
    ) {
        Err(error) => error,
        Ok(_) => panic!("an unusable E4B plan must exhaust the fallback"),
    };
    assert_eq!(error.code, "planner_output_unusable");
    assert_eq!(error.boundary, "AgentPlanning");
    assert_eq!(error.message, safe_message);
}

#[test]
fn incomplete_e4b_fallback_preserves_typed_cloud_exhaustion() {
    let safe_message = "The planning response did not match OOMU's required action format after one repair attempt. Your request is saved and no action was executed. Try again or choose another model.";
    let original_error = planner_output_unusable_error(safe_message);
    let fallback_target = cloud_planner_local_fallback_target(&original_error)
        .expect("schema exhaustion selects the local fallback");

    let error = match finalize_cloud_planner_local_fallback(
        "Create /tmp/supplier_decision.xlsx.",
        diagnostics_draft(),
        original_error,
        fallback_target,
    ) {
        Err(error) => error,
        Ok(_) => panic!("an incomplete E4B plan must exhaust the fallback"),
    };
    assert_eq!(error.code, "planner_output_unusable");
    assert_eq!(error.boundary, "AgentPlanning");
    assert_eq!(error.message, safe_message);
}

#[test]
fn verified_e4b_fallback_returns_its_effective_planner_target() {
    let original_error =
        planner_output_unusable_error("The cloud planner exhausted schema repair.");
    let fallback_target = cloud_planner_local_fallback_target(&original_error)
        .expect("schema exhaustion selects the local fallback");

    let (draft, effective_target) = finalize_cloud_planner_local_fallback(
        "Run local diagnostics.",
        diagnostics_draft(),
        original_error,
        fallback_target,
    )
    .expect("a complete E4B draft is accepted");

    assert_eq!(draft.steps.len(), 1);
    let metadata = effective_target.model_metadata();
    assert_eq!(metadata.name, "Gemma 4 E4B");
    assert_eq!(metadata.version, CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID);
    assert_eq!(metadata.locality, "local");
}
