use super::{
    append_execution_log, decision_pack_postcondition, release_recovery_postcondition, ActionPlan,
    AgentExecutionOriginGuard, AgenticLoopError,
};
use crate::{db::PersistenceEngine, shield_gate::ExecuteCommandResponse};

pub(super) async fn verify(
    plan: &ActionPlan,
    outputs: &[ExecuteCommandResponse],
    persistence: &PersistenceEngine,
    execution_id: Option<&str>,
    session_id: Option<&str>,
    agent_id: Option<&str>,
    app: Option<&tauri::AppHandle>,
    origin_guard: Option<&AgentExecutionOriginGuard>,
    execution_path: &mut Vec<String>,
) -> Result<(), AgenticLoopError> {
    if let Some(origin_guard) = origin_guard {
        origin_guard.ensure_current()?;
    }
    let decision_pack = decision_pack_postcondition::verify_if_required(
        plan,
        outputs,
        persistence,
        execution_id,
        app,
    )
    .await
    .map_err(|error| {
        append_execution_log(
            persistence,
            execution_id,
            &plan.id,
            session_id,
            agent_id,
            "error",
            "postcondition_failed",
            error.message.clone(),
            Some(serde_json::json!({
                "code": error.code,
                "boundary": error.boundary,
            })),
        );
        error
    })?;
    if let Some(origin_guard) = origin_guard {
        origin_guard.ensure_current()?;
    }
    let release_recovery = release_recovery_postcondition::verify_if_required(plan, outputs, app)
        .await
        .map_err(|error| {
            append_execution_log(
                persistence,
                execution_id,
                &plan.id,
                session_id,
                agent_id,
                "error",
                "postcondition_failed",
                error.message.clone(),
                Some(serde_json::json!({
                    "code": error.code,
                    "boundary": error.boundary,
                })),
            );
            error
        })?;
    if let Some(origin_guard) = origin_guard {
        origin_guard.ensure_current()?;
    }
    if let Some(postcondition) = decision_pack {
        append_execution_log(
            persistence,
            execution_id,
            &plan.id,
            session_id,
            agent_id,
            "info",
            "postcondition_verified",
            "Fresh cross-surface postcondition verification succeeded.",
            Some(postcondition.audit_payload),
        );
        execution_path.extend(postcondition.execution_path);
    }
    if let Some(postcondition) = release_recovery {
        append_execution_log(
            persistence,
            execution_id,
            &plan.id,
            session_id,
            agent_id,
            "info",
            "postcondition_verified",
            "Fresh recovery-agenda, Calendar, and Mail verification succeeded.",
            Some(postcondition.audit_payload),
        );
        execution_path.extend(postcondition.execution_path);
    }
    Ok(())
}
