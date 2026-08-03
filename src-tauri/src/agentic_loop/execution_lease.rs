use super::*;

pub(super) fn refresh_before_effectful_step(
    plan_approved: bool,
    potentially_effectful: bool,
    leases: Option<&ActuationLeaseManager>,
    app: Option<&tauri::AppHandle>,
    active_actor_id: &str,
    session_id: Option<&str>,
) -> Result<(), AgenticLoopError> {
    if !plan_approved || !potentially_effectful {
        return Ok(());
    }
    let leases = leases.ok_or_else(|| AgenticLoopError {
        code: "actuation_lease_unavailable",
        boundary: "ActuationLeaseManager",
        message: "Autonomous mutating action requires the Shield Gate actuation lease manager."
            .to_string(),
        mlc_path: None,
    })?;
    leases
        .refresh_for_continuation(
            app,
            active_actor_id,
            session_id.unwrap_or_default(),
            APPROVED_AGENT_PLAN_LEASE_DURATION_MS,
        )
        .map_err(|error| AgenticLoopError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
            mlc_path: None,
        })
        .map(|_| ())
}
