//! Post-window native startup work.
//!
//! This module intentionally runs on a blocking worker after Tauri's setup
//! callback returns. That keeps the native splash and macOS event loop
//! responsive while preserving the product contract that the startup model is
//! verified before the main OOMU window becomes available.

use crate::persistence_health::{BackingStoreClass, DegradedModeState};
use crate::{
    artifacts, auto_route_startup, background_runtime_lifecycle, background_tasks, db, gateway,
    gemma, knowledge, mcp, projects, redaction, settings,
    startup_splash::{StartupMilestone, StartupSplash},
    tasks, workflow_scheduler,
};
use tauri::Manager;

pub(crate) fn complete(
    app: tauri::AppHandle,
    startup_service: gemma::GemmaService,
    safe_mode: bool,
    splash: StartupSplash,
) {
    let setup_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let degraded_mode = app.state::<DegradedModeState>();
        probe_artifact_pipeline(&degraded_mode);
        if let Err(error) = tasks::reconcile_all(app.state::<db::PersistenceEngine>().inner()) {
            eprintln!(
                "TASK_CONTROL_PLANE_RECOVERY_FAILED {}",
                redaction::redacted_log_text(&error)
            );
        }
        attach_gateway(&app, &degraded_mode);
        probe_identity(&app, &degraded_mode);

        let _ = splash.report(&app, StartupMilestone::LoadingStartupModel);
        let startup_model_ready = prepare_startup_model(&app, &startup_service, &degraded_mode);
        let _ = splash.report(&app, StartupMilestone::FinishingStartup);

        background_runtime_lifecycle::reconcile_startup_handle(&app);
        bootstrap_mcp(&app, &degraded_mode);
        start_workflow_scheduler(&app, &degraded_mode);
        background_tasks::hooks::refresh_active_mod_hook_registry_async(
            app.clone(),
            app.state::<background_tasks::hooks::BackgroundHookRegistry>()
                .inner()
                .clone(),
            app.state::<db::PersistenceEngine>().inner().clone(),
            app.state::<gemma::GemmaService>().inner().clone(),
            app.state::<crate::sovereign_identity::SovereignIdentity>()
                .inner()
                .clone(),
            safe_mode,
        );
        startup_model_ready
    }));

    match setup_result {
        Ok(_) => {
            if let Err(error) = splash.reveal_main_when_ready(&app) {
                eprintln!("OOMU_STARTUP_MAIN_REVEAL_DISPATCH_FAILED error={error}");
            }
        }
        Err(payload) => {
            let error = gemma::GemmaError {
                code: "tauri_setup_panicked",
                message: format!(
                    "Tauri startup setup panicked and was safely contained: {}",
                    crate::panic_payload_message(payload)
                ),
            };
            eprintln!(
                "OOMU_STARTUP_SETUP_PANICKED code={} message={}",
                redaction::redacted_log_text(&error.code),
                redaction::redacted_log_text(&error.message)
            );
            startup_service.enter_degraded(error.clone());
            app.state::<DegradedModeState>().activate(
                "startup",
                crate::degraded_reason_from_error(&error),
                BackingStoreClass::NotApplicable,
                true,
                "Application startup did not complete safely.",
            );
            if let Err(dispatch_error) = splash.reveal_main_for_recovery(&app) {
                eprintln!("OOMU_STARTUP_RECOVERY_REVEAL_DISPATCH_FAILED error={dispatch_error}");
            }
        }
    }
}

fn probe_artifact_pipeline(degraded_mode: &DegradedModeState) {
    match artifacts::probe_pipeline_runtime() {
        Ok(()) => {
            degraded_mode.clear_after_verified_recovery(
                "artifactPipeline",
                BackingStoreClass::NotApplicable,
                "Packaged artifact builder and PDF renderer startup probes passed.",
            );
        }
        Err(error) => degraded_mode.activate(
            "artifactPipeline",
            &error,
            BackingStoreClass::NotApplicable,
            true,
            "Rebuild the packaged artifact builder and PDF renderer sidecars.",
        ),
    }
}

fn attach_gateway(app: &tauri::AppHandle, degraded_mode: &DegradedModeState) {
    match app
        .state::<gateway::SovereignGatewayService>()
        .attach_app_handle(app.clone())
    {
        Ok(()) => {
            degraded_mode.clear_after_verified_recovery(
                "gateway",
                BackingStoreClass::NotApplicable,
                "Gateway application handle, persistence, and worker refresh attached successfully.",
            );
        }
        Err(error) => {
            eprintln!(
                "SOVEREIGN_GATEWAY_RUNTIME_ATTACH_FAILED {}",
                redaction::redacted_log_text(&error)
            );
            degraded_mode.activate(
                "gateway",
                format!("Gateway runtime attach failed: {error}"),
                BackingStoreClass::NotApplicable,
                true,
                "Connected messaging channels are unavailable.",
            );
        }
    }
}

fn probe_identity(app: &tauri::AppHandle, degraded_mode: &DegradedModeState) {
    let identity = app.state::<crate::sovereign_identity::SovereignIdentity>();
    match identity.profile() {
        Ok(_) => {
            degraded_mode.clear_after_verified_recovery(
                "identity",
                BackingStoreClass::Persistent,
                "The cached root signing identity and public profile were verified.",
            );
        }
        Err(error) => degraded_mode.activate(
            "identity",
            format!("Secure identity health probe failed: {}", error.message),
            BackingStoreClass::RecoveryPending,
            true,
            "Signing and identity-backed operations are unavailable.",
        ),
    }
}

fn prepare_startup_model(
    app: &tauri::AppHandle,
    startup_service: &gemma::GemmaService,
    degraded_mode: &DegradedModeState,
) -> bool {
    let model_root = match settings::resolved_local_model_directory(app) {
        Ok(model_root) => model_root,
        Err(error) => {
            let error = gemma::GemmaError {
                code: "local_model_directory_unavailable",
                message: format!("Local model directory could not be resolved: {error}"),
            };
            mark_startup_model_failed(startup_service, degraded_mode, error);
            return false;
        }
    };
    let prewarm_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<gemma::StartupModelAssignment, gemma::GemmaError> {
            auto_route_startup::prepare(app, startup_service, &model_root)
        },
    ));
    match prewarm_result {
        Ok(Ok(assignment)) => {
            eprintln!(
                "STARTUP_MODEL_RESIDENT requested_model_id={} model_id={} selection_source={}",
                redaction::redacted_log_text(&assignment.requested_model_id),
                redaction::redacted_log_text(&assignment.resolved_model_id),
                assignment.selection_source.as_str(),
            );
            refresh_project_knowledge(app);
            true
        }
        Ok(Err(error)) => {
            mark_startup_model_failed(startup_service, degraded_mode, error);
            false
        }
        Err(payload) => {
            mark_startup_model_failed(
                startup_service,
                degraded_mode,
                gemma::GemmaError {
                    code: "local_infer_startup_panicked",
                    message: format!(
                        "Local inference startup panicked and was safely contained: {}",
                        crate::panic_payload_message(payload)
                    ),
                },
            );
            false
        }
    }
}

fn mark_startup_model_failed(
    startup_service: &gemma::GemmaService,
    degraded_mode: &DegradedModeState,
    error: gemma::GemmaError,
) {
    let reason = crate::degraded_reason_from_error(&error);
    startup_service.enter_degraded(error.clone());
    degraded_mode.activate(
        "autoRouteClassifier",
        reason,
        BackingStoreClass::NotApplicable,
        true,
        "Auto-route remains unavailable until its local classifier passes a real inference probe.",
    );
    eprintln!(
        "STARTUP_MODEL_PREPARATION_FAILED code={} message={}",
        redaction::redacted_log_text(error.code),
        redaction::redacted_log_text(&error.message)
    );
}

fn refresh_project_knowledge(app: &tauri::AppHandle) {
    let source_engine = app.state::<db::PersistenceEngine>().inner().clone();
    let source_knowledge = app.state::<knowledge::KnowledgeStore>().inner().clone();
    let source_gemma = app.state::<gemma::GemmaService>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        match projects::repository::refresh_active_knowledge_sources_at_startup(
            &source_engine,
            &source_knowledge,
            source_gemma,
        ) {
            Ok(summary) => eprintln!(
                "PROJECT_KNOWLEDGE_STARTUP_REFRESH refreshed={} empty={} failed={}",
                summary.refreshed, summary.empty, summary.failed
            ),
            Err(error) => eprintln!(
                "PROJECT_KNOWLEDGE_STARTUP_REFRESH_FAILED message={}",
                redaction::redacted_log_text(&error)
            ),
        }
    });
}

fn bootstrap_mcp(app: &tauri::AppHandle, degraded_mode: &DegradedModeState) {
    match mcp::bootstrap::bootstrap_mcp_runtime(app) {
        Ok(report) => {
            let registered_configs = tauri::async_runtime::block_on(
                app.state::<mcp::client::McpClientRegistry>()
                    .register_trusted_server_configs(report.server_configs.clone()),
            );
            eprintln!(
                "MCP_RUNTIME_READY python_runtime={} resource_root_resolved={} venv_root_resolved={} created_venv={} optional_python_error={} registered_configs={}",
                report.python_path.is_some(),
                report.resource_root.is_some(),
                report.venv_root.is_some(),
                report.created_venv,
                redaction::redacted_log_text(
                    report.optional_python_runtime_error.as_deref().unwrap_or("none")
                ),
                registered_configs
            );
            mcp::bootstrap::record_mcp_runtime_health(degraded_mode, &report);
        }
        Err(error) => {
            eprintln!(
                "MCP_RUNTIME_BOOTSTRAP_FAILED message={}",
                redaction::redacted_log_text(&error)
            );
            degraded_mode.activate(
                "mcpRuntime",
                format!("MCP runtime bootstrap failed: {error}"),
                BackingStoreClass::NotApplicable,
                true,
                "MCP tools are unavailable until bootstrap succeeds.",
            );
        }
    }
}

fn start_workflow_scheduler(app: &tauri::AppHandle, degraded_mode: &DegradedModeState) {
    match workflow_scheduler::spawn_background_worker(
        app.clone(),
        app.state::<db::PersistenceEngine>().inner().clone(),
        app.state::<gemma::GemmaService>().inner().clone(),
        app.state::<knowledge::KnowledgeStore>().inner().clone(),
        app.state::<mcp::client::McpClientRegistry>()
            .inner()
            .clone(),
        app.state::<gateway::SovereignGatewayService>()
            .inner()
            .clone(),
    ) {
        Ok(runtime) => {
            app.manage(runtime);
            degraded_mode.clear_after_verified_recovery(
                "workflowScheduler",
                BackingStoreClass::NotApplicable,
                "Workflow scheduler thread started successfully.",
            );
        }
        Err(error) => {
            eprintln!(
                "WORKFLOW_SCHEDULER_START_FAILED {}",
                redaction::redacted_log_text(&error)
            );
            degraded_mode.activate(
                "workflowScheduler",
                format!("Workflow scheduler could not start: {error}"),
                BackingStoreClass::NotApplicable,
                true,
                "Scheduled workflows are unavailable until the scheduler starts.",
            );
        }
    }
}
