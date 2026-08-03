use super::*;

pub(super) async fn save_before_permission(
    persistence: &PersistenceEngine,
    plan: &ActionPlan,
    step_index: usize,
) -> Result<(), AgenticLoopError> {
    persistence
        .save_plan_generation_state(
            plan.id.clone(),
            serialize_plan_for_persistence(plan)?,
            step_index,
            "awaiting_permission".to_string(),
            format!(
                "Saved progress before requesting permission for step {} of {}.",
                step_index + 1,
                plan.steps.len()
            ),
        )
        .await
        .map_err(AgenticLoopError::from_persistence)
}
