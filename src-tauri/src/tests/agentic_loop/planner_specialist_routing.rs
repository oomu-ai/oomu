use super::*;

#[test]
fn ordinary_manual_gemini_draft_keeps_legacy_alias_route_and_label() {
    let alias = selected_cloud_planner_target(None, Some("gemini"), Some("gemini-3.5-flash"))
        .expect("legacy alias resolves")
        .expect("legacy alias has a cloud target");
    let unchanged = bind_specialist_draft_provider_config(None, &diagnostics_draft(), alias)
        .expect("ordinary manual-cloud draft remains valid without a provider config");

    assert!(matches!(
        unchanged,
        PlannerExecutionTarget::Cloud(CloudPlannerTarget {
            provider_config_id: None,
            ref provider_id,
            ref provider_name,
            ..
        }) if provider_id == "gemini" && provider_name == "Google Gemini"
    ));
}

fn assert_objective_uses_ordinary_manual_gemini_route(objective: &str) {
    assert!(
        !plan_coverage::deterministic_draft_requires_dynamic_route(objective),
        "non-executable specialist mention required the specialist route: {objective}"
    );
    let (_, deterministic_draft) =
        plan_coverage::resolve_and_compile_decision_pack(objective.to_string(), None, false)
            .expect("ordinary objective remains plannable");
    assert!(
        deterministic_draft.is_none(),
        "non-executable specialist mention compiled an effectful draft: {objective}"
    );

    let target = resolve_planning_execution_target(
        None,
        objective,
        ModelRoutePreference::GeminiPro,
        Some("gemini"),
        Some("gemini-3.5-flash"),
    )
    .expect("ordinary manual Gemini alias remains available");
    assert!(matches!(
        target,
        PlannerExecutionTarget::Cloud(CloudPlannerTarget {
            provider_config_id: None,
            ref provider_id,
            ref provider_name,
            ..
        }) if provider_id == "gemini" && provider_name == "Google Gemini"
    ));

    let local_target = resolve_planning_execution_target(
        None,
        objective,
        ModelRoutePreference::LocalGemma,
        None,
        None,
    )
    .expect("ordinary local route remains available");
    assert!(matches!(local_target, PlannerExecutionTarget::Local { .. }));
}

#[test]
fn informational_and_negated_specialist_mentions_keep_the_ordinary_route() {
    for objective in [
        "How do OpenClaw and Claude Cowork differ for background work?",
        "What should a milestone recovery plan contain?",
        "Research current official OpenClaw and Claude Cowork background capabilities without writing `/Users/test/comparison.md`, then read it back only in chat.",
        "Read `/Users/test/milestones.json` and discuss a recovery plan respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and the requirement that security validation precede release validation. Do not write three failure contingencies to `/Users/test/recovery.md`.",
    ] {
        assert_objective_uses_ordinary_manual_gemini_route(objective);
    }
}

fn assert_manual_gemini_specialist_route(
    test_name: &str,
    objective: &str,
    expected_operation: &str,
) {
    let manager = temporary_agent_manager(test_name);
    let mut provider =
        planner_provider_config("prov-specialist-gemini", "google", "gemini-3.5-flash");
    provider.provider_name = "Configured Google route".to_string();
    provider.api_key_label = "GEMINI_API_KEY".to_string();
    provider.api_key = None;
    provider.credential_configured = false;
    manager
        .upsert_provider_config(provider)
        .expect("specialist provider config saves without opening credentials");

    let target = resolve_planning_execution_target(
        Some(&manager),
        objective,
        ModelRoutePreference::GeminiPro,
        Some("gemini"),
        Some("gemini-3.5-flash"),
    )
    .expect("manual specialist route resolves before approval");
    let (_, draft) =
        plan_coverage::resolve_and_compile_decision_pack(objective.to_string(), None, false)
            .expect("specialist objective compiles");
    let draft = draft.expect("specialist objective has a deterministic draft");
    assert!(matches!(
        &draft.steps[0].tool,
        GeneratedToolDraft::RegisteredTaskTool { operation, .. }
            if operation == expected_operation
    ));
    let unbound_alias =
        selected_cloud_planner_target(None, Some("gemini"), Some("gemini-3.5-flash"))
            .expect("legacy alias resolves")
            .expect("legacy alias has a cloud target");
    let draft_bound = bind_specialist_draft_provider_config(Some(&manager), &draft, unbound_alias)
        .expect("selected specialist tool binds an exact provider before preview");
    assert!(matches!(
        draft_bound,
        PlannerExecutionTarget::Cloud(CloudPlannerTarget {
            provider_config_id: Some(ref id),
            ..
        }) if id == "prov-specialist-gemini"
    ));
    let route = ModelRouter::route(
        objective,
        ModelRoutePreference::GeminiPro,
        &draft,
        0,
        &target,
    );

    assert_eq!(
        route.provider_config_id.as_deref(),
        Some("prov-specialist-gemini")
    );
    assert_eq!(route.provider_id.as_deref(), Some("google"));
    assert_eq!(route.selected_model.name, "gemini-3.5-flash");
    assert_eq!(route.selected_model.provider, "Google Gemini");
    assert_eq!(route.selected_model.locality, "remote");
}

#[test]
fn manual_gemini_comparison_binds_exact_provider_before_specialist_plan_approval() {
    assert_manual_gemini_specialist_route(
        "manual-comparison-specialist-route",
        "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison to `/Users/test/project/output/comparison.md` in my Project folder. Include URLs, access times, explicit limitations, and a section explaining what this implies for OOMU. Do not claim completion until the file exists and you have read it back.",
        crate::tools::evidence_artifacts::COMPARISON_OPERATION,
    );
}

#[test]
fn manual_gemini_milestone_binds_exact_provider_before_specialist_plan_approval() {
    assert_manual_gemini_specialist_route(
        "manual-milestone-specialist-route",
        "Read `/Users/test/project/milestone_source.json` and construct a recovery plan that minimizes completion time while respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and the requirement that security validation precede release validation. Write the assumptions, critical path, and three failure contingencies to `/Users/test/project/output/recovery.md` and verify the file.",
        crate::tools::evidence_artifacts::RECOVERY_OPERATION,
    );
}

#[test]
fn manual_gemini_specialist_without_exact_config_fails_before_plan_approval() {
    let result = resolve_planning_execution_target(
        None,
        "Research current official OpenClaw and Claude Cowork background capabilities, write `/Users/test/comparison.md`, and read it back.",
        ModelRoutePreference::GeminiPro,
        Some("gemini"),
        Some("gemini-3.5-flash"),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("specialist execution cannot defer exact-provider failure until execution"),
    };

    assert_eq!(error.code, "planner_provider_configuration_failed");
    assert_eq!(error.boundary, "AgentPlanning");
    assert!(error.message.contains("Choose a configured provider"));
}
