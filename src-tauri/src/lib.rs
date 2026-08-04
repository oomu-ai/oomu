macro_rules! eprintln {
    ($($arg:tt)*) => {
        $crate::diagnostic_output::write_diagnostic_line(format_args!($($arg)*))
    };
}
mod agent_manager;
mod agentic_loop;
mod airlock;
mod analysis;
mod app_shell;
mod app_updates;
mod approval_scopes;
mod artifact_auditor;
mod artifact_builder;
pub mod artifacts;
mod audit;
mod authority;
mod auto_route_startup;
mod background_runtime_lifecycle;
mod background_runtime_tray;
mod background_tasks;
pub mod browser_automation;
mod browser_proxy;
mod calendar_permissions;
mod capability_bundles;
mod chat_attention;
mod chat_session_lifecycle;
mod command_registration;
pub mod computer_use;
mod condition_expression;
pub mod connectors;
mod context_manager;
pub mod db;
mod decision_pack;
mod decision_research_policy;
pub mod delegation;
mod diagnostic_output;
pub(crate) use diagnostic_output::debug_trace_enabled;
mod dom_sanitizer;
mod dom_stream_commands;
mod dom_streaming;
mod errors;
#[path = "tools/eventkit_calendar.rs"]
pub(crate) mod eventkit_calendar;
mod file_export;
pub mod foundation;
mod gateway;
pub mod gemma;
mod hero_workflow;
mod inference;
mod keychain_namespace;
mod keychain_session;
mod knowledge;
mod launch_startup;
mod learning;
mod local_app_intent;
mod local_context;
mod local_context_drag;
mod mac_speech;
mod macos_permission_broker;
mod macos_process_identity;
mod mail_permissions;
pub mod mcp;
pub mod mcp_result;
mod media;
mod memory_ledger;
mod metal_backend;
mod native_app_ports;
mod native_browser;
mod native_capability_adapters;
mod native_menu;
mod native_runtime;
pub mod network_policy;
pub mod p0_contracts;
pub mod p1_contracts;
pub mod pdf_containment;
mod pdf_protocol;
mod persistence_health;
mod persistence_recovery_commands;
mod privacy;
mod production_task_tools;
pub mod projects;
mod recommended_model_activation;
mod recommended_model_install;
pub mod redaction;
mod remote_access;
mod remote_routines;
pub mod routines;
mod runtime_profile;
mod runtime_window_lifecycle; // Only the main window owns process-wide runtime teardown.
mod scenario_one_e2e_profile;
mod schedule_expression;
mod secret_store;
pub mod security;
pub mod settings;
mod shield_gate;
mod single_instance_contract;
pub mod sovereign_identity;
mod sovereign_search;
mod startup_integrity_ui;
mod startup_runtime;
mod startup_splash;
mod sys_info;
mod system_diagnostics;
mod system_music;
mod system_photos;
mod taskflow;
pub mod tasks;
pub mod tool_security;
mod tools;
mod verifier;
mod workflow_compiler;
pub mod workflow_ir;
pub mod workflow_runtime;
mod workflow_scheduler;
pub use app_shell::{parse_launch_options, OomuLaunchOptions};
pub(crate) use background_runtime_tray::{refresh_background_tray_menu, sync_background_tray};
use errors::OomuError;
use local_context_drag::emit_local_context_drag;
use persistence_health::{
    BackingStoreClass, DegradedModeState, DegradedModeStatus, VolatileRecoveryStatus,
    VolatileStoreSession, VolatileStoreSessionManager,
};
use {std::time::Duration, tauri::Manager};
fn update_window_icon(window: &tauri::WebviewWindow, theme: &tauri::Theme) {
    let icon_filename = match theme {
        tauri::Theme::Dark => "OOMU-macOS-Dark-1024x1024@1x.png",
        _ => "OOMU-macOS-Default-1024x1024@1x.png",
    };
    let packaged_icon = window
        .app_handle()
        .path()
        .resource_dir()
        .ok()
        .map(|directory| directory.join("icons").join(icon_filename));
    #[cfg(debug_assertions)]
    let candidate_paths = packaged_icon.into_iter().chain([
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("icons")
            .join(icon_filename),
        std::path::PathBuf::from("src-tauri/icons").join(icon_filename),
    ]);
    #[cfg(not(debug_assertions))]
    let candidate_paths = packaged_icon.into_iter();
    for path in candidate_paths {
        if let Ok(icon_image) = tauri::image::Image::from_path(&path) {
            if window.set_icon(icon_image).is_ok() {
                return;
            }
        }
    }
}

fn update_window_icon_for_event(window: &tauri::Window, theme: &tauri::Theme) {
    if let Some(webview_window) = window.app_handle().get_webview_window(window.label()) {
        update_window_icon(&webview_window, theme);
    }
}

fn emit_launch_debug_trace(options: &OomuLaunchOptions) {
    if !debug_trace_enabled() {
        return;
    }
    eprintln!(
        "OOMU_DEBUG_TRACE_READY profile={} debug_mode={} safe_mode={} first_run_setup={} reset_state={} dump_db={}",
        options.log_level,
        options.debug_mode,
        options.safe_mode,
        options.first_run_setup,
        options.reset_state,
        options.dump_db
    );
}
fn degraded_reason_from_error(error: &gemma::GemmaError) -> String {
    format!("{} ({})", error.message, error.code)
}
fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}
fn shutdown_runtime_services_once(
    coordinator: &runtime_window_lifecycle::RuntimeShutdownCoordinator,
    shutdown_service: &gemma::GemmaService,
    app_handle: &tauri::AppHandle,
) -> runtime_window_lifecycle::ShutdownReport {
    runtime_window_lifecycle::shutdown_runtime_services_once(
        coordinator,
        shutdown_service,
        app_handle,
    )
}
fn initialize_database_with_degraded_fallback<T, F, G>(
    degraded_mode: &DegradedModeState,
    volatile_sessions: &VolatileStoreSessionManager,
    subsystem: &str,
    boundary: &str,
    user_visible_impact: &str,
    initialize: F,
    initialize_fallback: G,
) -> Result<T, OomuError>
where
    F: FnOnce() -> Result<T, String>,
    G: FnOnce(&VolatileStoreSession) -> Result<T, String>,
{
    match initialize() {
        Ok(value) => {
            degraded_mode.register_healthy(
                subsystem,
                BackingStoreClass::Persistent,
                user_visible_impact,
            );
            Ok(value)
        }
        Err(primary_error) => {
            let reason = format!(
                "{boundary} unavailable; running with temporary degraded storage: {primary_error}"
            );
            eprintln!(
                "OOMU_DEGRADED_STARTUP boundary={} message={}",
                crate::redaction::redacted_log_text(boundary),
                crate::redaction::redacted_log_text(&primary_error)
            );
            degraded_mode.activate(
                subsystem,
                reason,
                BackingStoreClass::Volatile,
                true,
                user_visible_impact,
            );
            let session = volatile_sessions.get_or_create().map_err(|fallback_error| {
                OomuError::Database(format!(
                    "{boundary} initialization failed: {primary_error}; private volatile session allocation failed: {fallback_error}"
                ))
            })?;
            let value = initialize_fallback(&session).map_err(|fallback_error| {
                OomuError::Database(format!(
                    "{boundary} initialization failed: {primary_error}; degraded fallback failed: {fallback_error}"
                ))
            })?;
            session.enforce_private_tree().map_err(|error| {
                OomuError::Database(format!(
                    "{boundary} volatile storage privacy verification failed: {error}"
                ))
            })?;
            Ok(value)
        }
    }
}
fn initialize_required_database<T, F>(
    degraded_mode: &DegradedModeState,
    subsystem: &str,
    boundary: &str,
    user_visible_impact: &str,
    initialize: F,
) -> Result<T, OomuError>
where
    F: FnOnce() -> Result<T, String>,
{
    match initialize() {
        Ok(value) => {
            degraded_mode.register_healthy(
                subsystem,
                BackingStoreClass::Persistent,
                user_visible_impact,
            );
            Ok(value)
        }
        Err(error) => {
            degraded_mode.activate(
                subsystem,
                format!("{boundary} durable initialization failed: {error}"),
                BackingStoreClass::RecoveryPending,
                false,
                user_visible_impact,
            );
            Err(OomuError::Database(format!(
                "{boundary} refuses an unencrypted volatile fallback: {error}"
            )))
        }
    }
}
mod lib_commands;
use lib_commands::{
    degraded_mode_commands, launch_options_commands, local_inference_recovery_commands,
};
#[tauri::command]
async fn stream_execution_steps(
    execution_id: String,
    last_seen_id: Option<i64>,
    channel: tauri::ipc::Channel<db::AgentExecutionLogBatch>,
    persistence: tauri::State<'_, db::PersistenceEngine>,
) -> Result<(), agentic_loop::AgenticLoopError> {
    let execution_id = execution_id.trim().to_string();
    if execution_id.is_empty() {
        return Err(agentic_loop::AgenticLoopError {
            code: "execution_id_required",
            boundary: "ExecutionStream",
            message: "Execution stream requires an execution_id.".to_string(),
            mlc_path: None,
        });
    }

    let engine = persistence.inner().clone();
    let mut cursor = last_seen_id.unwrap_or_default().max(0);
    tauri::async_runtime::spawn(async move {
        let mut idle_ticks = 0_u32;
        loop {
            let query_engine = engine.clone();
            let query_execution_id = execution_id.clone();
            let logs = match tauri::async_runtime::spawn_blocking(move || {
                query_engine.select_agent_execution_logs_after(&query_execution_id, cursor, 100)
            })
            .await
            {
                Ok(Ok(logs)) => logs,
                Ok(Err(error)) => {
                    eprintln!(
                        "OOMU_AGENT_EXECUTION_STREAM_QUERY_FAILED execution_id={} error={}",
                        execution_id, error
                    );
                    break;
                }
                Err(error) => {
                    eprintln!(
                        "OOMU_AGENT_EXECUTION_STREAM_JOIN_FAILED execution_id={} error={}",
                        execution_id, error
                    );
                    break;
                }
            };

            if logs.is_empty() {
                idle_ticks = idle_ticks.saturating_add(1);
                if idle_ticks > 4_500 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
                continue;
            }

            idle_ticks = 0;
            if let Some(last) = logs.last() {
                cursor = last.id;
            }
            let terminal = logs.iter().any(db::AgentExecutionLogRecord::is_terminal);
            let batch = db::AgentExecutionLogBatch {
                execution_id: execution_id.clone(),
                logs,
                terminal,
            };
            if channel.send(batch).is_err() || terminal {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    Ok(())
}

pub(crate) fn refresh_oomu_menu(
    app: &tauri::AppHandle,
    translations: Option<&serde_json::Value>,
) -> tauri::Result<()> {
    native_menu::refresh(app, translations)
}
fn resolve_gateway_routine_approval(
    persistence: &db::PersistenceEngine,
    gemma: gemma::GemmaService,
    app: tauri::AppHandle,
    instance_id: String,
    approval_token: String,
    approve: bool,
) -> Result<(), String> {
    workflow_scheduler::resolve_scheduled_permission(
        workflow_runtime::ResolvePermissionRequest {
            instance_id,
            approval_token,
            decision: if approve {
                workflow_runtime::PermissionDecision::Approve
            } else {
                workflow_runtime::PermissionDecision::Reject
            },
        },
        persistence,
        gemma,
        app.state::<mcp::client::McpClientRegistry>()
            .inner()
            .clone(),
        app,
    )
    .map(|_| ())
    .map_err(|error| error.message)
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if routines::background::run_worker_if_requested() {
        return;
    }
    startup_splash::begin_launch_timing();
    let launched = diagnostic_output::prepare_launch(parse_launch_options())
        .map_err(OomuError::Startup)
        .and_then(try_run);
    if let Err(error) = launched {
        startup_splash::dismiss_active_for_failure();
        startup_integrity_ui::show(&error);
        eprintln!(
            "OOMU_STARTUP_FAILED code={} boundary={} message={}",
            crate::redaction::redacted_log_text(error.code()),
            crate::redaction::redacted_log_text(error.boundary()),
            crate::redaction::redacted_log_text(&error.message())
        );
    }
}
fn try_run(launch_opts: OomuLaunchOptions) -> Result<(), OomuError> {
    launch_startup::configure_launch_logging_profile(&launch_opts);
    emit_launch_debug_trace(&launch_opts);
    launch_startup::exit_after_help_if_requested(&launch_opts);

    let Some(startup_authority) = launch_startup::establish_startup_authority(&launch_opts)? else {
        return Ok(());
    };
    let startup_instance_identity = startup_authority.identity();
    tasks::register_runtime_bridge().map_err(OomuError::Startup)?;
    production_task_tools::register_production_task_tools()?;
    if launch_opts.dump_db {
        println!("OOMU STATE LEDGER DIAGNOSTIC DUMP");
        match db::execute_terminal_db_audit() {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!(
                    "Failed to run database audit: {}",
                    crate::redaction::redacted_log_text(&error.to_string())
                );
                std::process::exit(1);
            }
        }
    }
    let startup_splash = startup_authority.require_gui_splash();
    if launch_opts.reset_state {
        println!("Reset state flag detected. Purging transient caches...");
        if let Err(error) = db::purge_transient_sqlite_cache() {
            eprintln!(
                "Warning: Failed to clear local state cache: {}",
                crate::redaction::redacted_log_text(&error.to_string())
            );
        }
    }
    let degraded_mode_status = DegradedModeState::default();
    let volatile_store_sessions = VolatileStoreSessionManager::initialize().map_err(|error| {
        OomuError::Database(format!("Volatile recovery discovery failed: {error}"))
    })?;
    if volatile_store_sessions.current().is_some() {
        degraded_mode_status.activate(
            "chatSessionPersistence",
            "A private encrypted recovery session from an earlier launch requires reconciliation or export.",
            BackingStoreClass::RecoveryPending,
            true,
            "Earlier chat/session writes remain in encrypted recovery storage until explicitly reconciled and cleaned up.",
        );
    }
    let gemma_service = gemma::GemmaService::new_loading();
    if let Some(reason) = gemma_service.degraded_reason() {
        degraded_mode_status.activate(
            "autoRouteClassifier",
            format!("Auto-route classifier runtime unavailable: {reason}"),
            BackingStoreClass::NotApplicable,
            true,
            "Auto-route remains unavailable until its local classifier passes a real inference probe.",
        );
    }
    let shutdown_service = gemma_service.clone();
    let window_shutdown_service = shutdown_service.clone();
    let shutdown_coordinator = runtime_window_lifecycle::RuntimeShutdownCoordinator::default();
    let window_shutdown_coordinator = shutdown_coordinator.clone();
    let startup_service = gemma_service.clone();
    let safe_mode = launch_opts.safe_mode;
    let knowledge_store = initialize_required_database(
        &degraded_mode_status,
        "knowledge",
        "KnowledgeStore",
        "Knowledge storage is unavailable; plaintext temporary indexing is forbidden.",
        knowledge::KnowledgeStore::initialize,
    )?;
    let sovereign_identity = launch_startup::initialize_sovereign_identity()?;
    degraded_mode_status.register_healthy(
        "identity",
        BackingStoreClass::Persistent,
        "Secure identity is available.",
    );
    let pre_alpha_audit = initialize_required_database(
        &degraded_mode_status,
        "audit",
        "PreAlphaAudit",
        "Release-critical and irreversible operations are blocked because audit storage is unavailable.",
        audit::PreAlphaAudit::initialize,
    )?;
    let agent_manager = initialize_required_database(
        &degraded_mode_status,
        "agent",
        "AgentManager",
        "Agent configuration storage is unavailable; temporary fallback is forbidden.",
        agent_manager::AgentManager::initialize,
    )?;
    launch_startup::configure_scenario_agent_manager(&agent_manager)?;
    let memory_ledger = initialize_required_database(
        &degraded_mode_status,
        "memory",
        "MemoryLedger",
        "Memory storage is unavailable; temporary fallback is forbidden.",
        memory_ledger::MemoryLedger::initialize,
    )?;
    let recovery_ledger = memory_ledger.clone();
    let taskflow_engine = initialize_required_database(
        &degraded_mode_status,
        "taskFlow",
        "TaskFlowEngine",
        "TaskFlow storage is unavailable; temporary execution fallback is forbidden.",
        taskflow::TaskFlowEngine::initialize,
    )?;
    let recovery_taskflow_engine = taskflow_engine.clone();
    let persistence = initialize_database_with_degraded_fallback(
        &degraded_mode_status,
        &volatile_store_sessions,
        "chatSessionPersistence",
        "PersistentStateEngine",
        "Chats, queues, settings, and workflow state are being written to private volatile storage.",
        db::PersistenceEngine::initialize,
        |session| db::PersistenceEngine::initialize_volatile_at(session.path_for_file("state")?),
    )?;
    if persistence.storage_class() == BackingStoreClass::Persistent
        && volatile_store_sessions.current().is_some()
    {
        match persistence_recovery_commands::reconcile_non_conflicting_sessions_at_startup(
            &persistence,
            &degraded_mode_status,
            &volatile_store_sessions,
        ) {
            Ok(result) if result.recovered_sessions > 0 => eprintln!(
                "OOMU_RECOVERY_AUTO_RECONCILED status={} sessions={}",
                if result.fully_drained {
                    "verified_all_non_conflicting"
                } else {
                    "paused_before_conflict"
                },
                result.recovered_sessions
            ),
            Ok(_) => {}
            Err(error) => eprintln!(
                "OOMU_RECOVERY_AUTO_RECONCILE_SKIPPED message={}",
                crate::redaction::redacted_log_text(&error)
            ),
        }
    }
    gemma_service.attach_audit_persistence(persistence.clone());
    if launch_opts.safe_mode {
        eprintln!("OOMU_SAFE_MODE_ACTIVE third_party_mods=blocked dynamic_routing=manual");
        if let Err(error) = persistence.apply_safe_mode_boot_rules() {
            eprintln!(
                "OOMU_SAFE_MODE_BOOT_RULES_FAILED {}",
                crate::redaction::redacted_log_text(&error.to_string())
            );
        }
    }
    agent_manager.audit_recovery();
    taskflow_engine.audit_orphans();
    recovery_taskflow_engine.spawn_recovery(
        sovereign_identity.clone(),
        recovery_ledger,
        gemma_service.clone(),
        persistence.clone(),
    );
    persistence.audit_recovery();
    gateway::register_routine_approval_resolver(resolve_gateway_routine_approval)
        .map_err(OomuError::Startup)?;
    let gateway_service = gateway::SovereignGatewayService::initialize(
        persistence.clone(),
        agent_manager.clone(),
        knowledge_store.clone(),
        memory_ledger.clone(),
        sovereign_identity.clone(),
        gemma_service.clone(),
        launch_opts.safe_mode,
    );
    degraded_mode_status.activate(
        "gateway",
        "Gateway service initialized; application-handle and persistence attachment probe has not completed.",
        BackingStoreClass::NotApplicable,
        true,
        "Connected messaging channels remain unavailable until gateway attachment succeeds.",
    );
    degraded_mode_status.activate(
        "mcpRuntime",
        "MCP runtime bootstrap probe has not completed.",
        BackingStoreClass::NotApplicable,
        true,
        "MCP runtime startup is pending its bootstrap probe.",
    );
    degraded_mode_status.activate(
        "workflowScheduler",
        "Workflow scheduler thread probe has not completed.",
        BackingStoreClass::NotApplicable,
        true,
        "Workflow scheduler startup is pending its thread probe.",
    );
    degraded_mode_status.activate(
        "backgroundHooks",
        "Background hook registry refresh probe has not completed.",
        BackingStoreClass::NotApplicable,
        true,
        "Background hook registry startup is pending its refresh probe.",
    );
    let page_load_splash = startup_splash.clone();
    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(app_updates::updater_public_key())
                .build(),
        )
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(launch_startup::opener_plugin())
        .plugin(tauri_plugin_shell::init())
        .menu(native_menu::build)
        .on_page_load(move |webview, payload| {
            app_shell::scenario_one_ui_driver::on_page_load(webview, payload);
            if webview.label() == "main"
                && payload.event() == tauri::webview::PageLoadEvent::Finished
            {
                if let Err(error) = page_load_splash.mark_main_shell_ready(webview.app_handle()) {
                    eprintln!("OOMU_STARTUP_SHELL_READY_DISPATCH_FAILED error={error}");
                }
            }
        })
        .on_menu_event(native_menu::handle_event)
        .manage(launch_opts.clone())
        .manage(degraded_mode_status)
        .manage(volatile_store_sessions)
        .manage(agent_manager)
        .manage(memory_ledger)
        .manage(pre_alpha_audit)
        .manage(knowledge_store)
        .manage(taskflow_engine)
        .manage(sovereign_identity)
        .manage(gemma_service)
        .manage(computer_use::AppControlManager::production(
            persistence.clone(),
        ))
        .manage(persistence)
        .manage(gateway_service)
        .manage(gateway::auto_turn::AutoTurnRegistry::default())
        .manage(knowledge::KnowledgeIngestGrantStore::default())
        .manage(local_context::LocalContextGrantStore::default())
        .manage(native_browser::NativeBrowserManager::default())
        .manage(browser_automation::BrowserAutomationManager::default())
        .manage(browser_automation::BrowserTransferManager::default())
        .manage(artifacts::ArtifactRuntimeManager::default())
        .manage(artifacts::presentations::PresentationExportGrantStore::default())
        .manage(delegation::DelegationRuntime::default())
        .manage(background_tasks::hooks::BackgroundHookRegistry::default())
        .manage(shield_gate::ShieldApprovalManager::default())
        .manage(shield_gate::ScopeTrustManager::default())
        .manage(shield_gate::ActuationLeaseManager::default())
        .manage(authority::NativeAuthorityManager::default())
        .manage(app_updates::ApplicationUpdateService::default())
        .manage(mcp::client::McpClientRegistry::default())
        .manage(mac_speech::VoiceCaptureManager::default())
        .manage(routines::BackgroundRuntimeSupervisor::default())
        .setup(move |app| {
            use tauri::Manager;
            #[cfg(target_os = "macos")]
            launch_startup::install_single_instance_activation_listener(
                app,
                startup_instance_identity.clone(),
            )
            .map_err(|error| format!("Single-instance activation setup failed: {error}"))?;
            if let Some(window) = app.get_webview_window("main") {
                let theme = window.theme().unwrap_or(tauri::Theme::Light);
                update_window_icon(&window, &theme);
            }
            let app_handle = app.handle().clone();
            app.manage(recommended_model_install::RecommendedModelInstaller::new(
                settings::models_root(),
                settings::app_data_root(),
                recommended_model_activation::AppRecommendedModelFinalizer::new(app_handle.clone()),
            ));
            let worker_startup_service = startup_service.clone();
            let worker_splash = startup_splash.clone();
            tauri::async_runtime::spawn_blocking(move || {
                startup_runtime::complete(
                    app_handle,
                    worker_startup_service,
                    safe_mode,
                    worker_splash,
                );
            });
            Ok(())
        })
        .on_webview_event(|webview, event| {
            if webview.label() == "main" {
                if let tauri::WebviewEvent::DragDrop(drag_event) = event {
                    emit_local_context_drag(&webview.window(), drag_event);
                }
            }
        })
        .on_window_event(move |window, event| match event {
            tauri::WindowEvent::ThemeChanged(theme) => update_window_icon_for_event(window, theme),
            tauri::WindowEvent::DragDrop(drag_event) if window.label() == "main" => {
                // Non-unstable Tauri builds synthesize the same drop as a
                // window event. The shared handler keeps both runtime modes
                // correct without exposing host paths to the renderer.
                emit_local_context_drag(window, drag_event);
            }
            tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                api.prevent_close();
                if background_runtime_lifecycle::hide_main_window_if_verified(window)
                    != background_runtime_lifecycle::CloseRequestDisposition::ExitApplication
                {
                    return;
                }
                shutdown_runtime_services_once(
                    &window_shutdown_coordinator,
                    &window_shutdown_service,
                    window.app_handle(),
                );
                window.app_handle().exit(0);
            }
            tauri::WindowEvent::Destroyed
                if runtime_window_lifecycle::requires_runtime_shutdown(window.label()) =>
            {
                shutdown_runtime_services_once(
                    &window_shutdown_coordinator,
                    &window_shutdown_service,
                    window.app_handle(),
                );
            }
            _ => {}
        })
        .invoke_handler(command_registration::oomu_command_handler!())
        .build(tauri::generate_context!())
        .map_err(|error| OomuError::Startup(format!("Tauri application build failed: {error}")))?;
    app.run(move |_app_handle, event| match event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            background_runtime_tray::restore_foreground(_app_handle);
        }
        tauri::RunEvent::ExitRequested { .. } => {
            shutdown_runtime_services_once(&shutdown_coordinator, &shutdown_service, _app_handle);
        }
        tauri::RunEvent::Exit => {
            shutdown_runtime_services_once(&shutdown_coordinator, &shutdown_service, _app_handle);
        }
        _ => {}
    });
    Ok(())
}
