use super::*;

pub(super) struct ModelRouter;

impl ModelRouter {
    pub(super) fn route(
        _objective: &str,
        _preference: ModelRoutePreference,
        _draft: &GeneratedActionPlanDraft,
        context_excerpt_count: usize,
        planner_target: &PlannerExecutionTarget,
    ) -> ModelRouteDecision {
        let _routing_diagnostic = planner_target.routing_diagnostic();
        let selected_model = planner_target.model_metadata();
        let reason = plain_plan_route_reason(&selected_model, context_excerpt_count, false);
        ModelRouteDecision {
            selected_model,
            provider_config_id: provider_config_id(planner_target),
            provider_id: provider_id(planner_target),
            recommended_model: None,
            requires_principal_authorization: false,
            reason,
            context_excerpt_count,
            context_sources: Vec::new(),
        }
    }
}

fn provider_config_id(target: &PlannerExecutionTarget) -> Option<String> {
    match target {
        PlannerExecutionTarget::Local { .. } => None,
        PlannerExecutionTarget::Cloud(target) => target.provider_config_id.clone(),
    }
}

fn provider_id(target: &PlannerExecutionTarget) -> Option<String> {
    match target {
        PlannerExecutionTarget::Local { .. } => Some("local_model".to_string()),
        PlannerExecutionTarget::Cloud(target) => Some(
            target
                .provider_id
                .trim()
                .to_ascii_lowercase()
                .replace('-', "_"),
        ),
    }
}
