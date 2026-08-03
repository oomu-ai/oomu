use super::ActionPlan;

pub(crate) fn matches_scenario_one_deterministic_plan(
    plan: &ActionPlan,
    trusted_output_directory: &str,
) -> bool {
    super::plan_coverage::matches_deterministic_decision_pack_plan(plan, trusted_output_directory)
}
