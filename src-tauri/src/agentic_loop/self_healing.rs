use super::*;

pub(super) async fn compile_self_healing_plan(
    failed_plan: &ActionPlan,
    root_objective: &str,
    directive: &str,
    attempt: usize,
    gemma: GemmaService,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
    web_search_enabled: bool,
) -> Result<ActionPlan, AgenticLoopError> {
    let objective = format!(
        "{directive}\n\nOriginal objective:\n{}\n\nCorrective attempt: {attempt}.",
        compact_for_prompt(root_objective, 2_000)
    );
    let context = build_project_context(&objective);
    let planning_sections =
        basic_planner_prompt_sections(&objective, &context, ModelRoutePreference::LocalGemma);
    let planner_target = PlannerExecutionTarget::Local {
        model_id: None,
        reason: format!(
            "Self-healing planner selected local corrective routing for parent plan {} attempt {attempt}.",
            failed_plan.id
        ),
    };
    let (draft, planner_target) =
        generate_plan_draft(objective.clone(), planning_sections, gemma, planner_target).await?;
    let draft = normalize_web_search_plan_draft(&objective, draft, web_search_enabled);
    let draft = normalize_generated_plan_for_known_objectives(&objective, draft);
    validate_planner_draft_for_execution(&objective, &draft, web_search_enabled)?;
    plan_coverage::validate_connected_service_bindings(&objective, &draft, persistence, None)?;
    let mut route = ModelRouter::route(
        &objective,
        ModelRoutePreference::LocalGemma,
        &draft,
        context.excerpts.len(),
        &planner_target,
    );
    route.context_sources = context.claim_sources.clone();
    route.reason =
        plain_plan_route_reason(&route.selected_model, route.context_excerpt_count, false);
    sign_plan(
        generated_draft_to_plan(objective, draft, route, context),
        identity,
    )
}
