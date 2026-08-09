use std::path::Path;
use tauri::Manager;

pub(crate) fn prepare(
    app: &tauri::AppHandle,
    service: &crate::gemma::GemmaService,
    model_root: &Path,
) -> Result<crate::gemma::StartupModelAssignment, crate::gemma::GemmaError> {
    let preference =
        crate::settings::resolved_startup_model_preference(app).map_err(|message| {
            crate::gemma::GemmaError {
                code: "startup_model_preference_lookup_failed",
                message,
            }
        })?;
    let assignment = crate::gemma::model_resolution::resolve_verified_startup_model_assignment(
        model_root,
        &preference,
    )?;
    eprintln!(
        "AUTO_ROUTE_STARTUP_ASSIGNMENT requested_model_id={} resolved_model_id={} selection_source={}",
        crate::redaction::redacted_log_text(&assignment.requested_model_id),
        crate::redaction::redacted_log_text(&assignment.resolved_model_id),
        assignment.selection_source.as_str(),
    );
    crate::diagnostic_output::write_diagnostic_line(format_args!(
        "OOMU_NATIVE_RECEIPT {}",
        serde_json::json!({
            "kind": "auto_route_startup_assignment",
            "requestedModelId": assignment.requested_model_id,
            "resolvedModelId": assignment.resolved_model_id,
            "selectionSource": assignment.selection_source,
        })
    ));
    service.load_startup_model_assignment(assignment.clone())?;
    reconcile_saved_model_authorities(app, model_root, &assignment)?;
    Ok(assignment)
}

fn reconcile_saved_model_authorities(
    app: &tauri::AppHandle,
    model_root: &Path,
    assignment: &crate::gemma::StartupModelAssignment,
) -> Result<(), crate::gemma::GemmaError> {
    let agent_manager = app.state::<crate::agent_manager::AgentManager>();
    let persistence = app.state::<crate::db::PersistenceEngine>();
    let degraded = app.state::<crate::persistence_health::DegradedModeState>();
    let result = persistence.reconcile_canonical_model_authorities(
        agent_manager.inner(),
        model_root,
        assignment,
    );
    match result {
        Ok(report) => {
            degraded.clear_after_verified_recovery(
                "autoRouteSessionBaselines",
                crate::persistence_health::BackingStoreClass::Persistent,
                "Saved Auto-route model choices were verified.",
            );
            eprintln!(
                "AUTO_ROUTE_AUTHORITIES_RECONCILED agents={} inspected={} preserved={} repaired={} needs_user_choice={}",
                report.aligned_agents, report.sessions.inspected, report.sessions.preserved,
                report.sessions.repaired, report.sessions.needs_user_choice,
            );
            Ok(())
        }
        Err(error) => {
            degraded.activate(
                "autoRouteSessionBaselines",
                format!("Saved Auto-route model verification failed: {error}"),
                crate::persistence_health::BackingStoreClass::Persistent,
                true,
                "Some saved chats need their on-device model checked before Auto-route can continue.",
            );
            Err(crate::gemma::GemmaError {
                code: "canonical_model_authority_migration_failed",
                message: error,
            })
        }
    }
}
