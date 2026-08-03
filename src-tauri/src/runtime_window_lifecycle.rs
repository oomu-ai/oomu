use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tauri::Manager;

const SHUTDOWN_OBSERVER_WAIT: Duration = Duration::from_secs(8);

pub(super) fn requires_runtime_shutdown(window_label: &str) -> bool {
    window_label == "main"
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShutdownPhaseReceipt {
    pub(crate) phase: &'static str,
    pub(crate) outcome: &'static str,
    pub(crate) detail_code: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShutdownReport {
    pub(crate) completed: bool,
    pub(crate) phases: Vec<ShutdownPhaseReceipt>,
}

impl ShutdownReport {
    pub(crate) fn run_phase(
        &mut self,
        phase: &'static str,
        operation: impl FnOnce() -> Result<(), String>,
    ) {
        let result = operation();
        self.phases.push(ShutdownPhaseReceipt {
            phase,
            outcome: if result.is_ok() {
                "completed"
            } else {
                "failed"
            },
            detail_code: result.err(),
        });
    }

    fn panicked() -> Self {
        Self {
            completed: true,
            phases: vec![ShutdownPhaseReceipt {
                phase: "shutdown_coordinator",
                outcome: "failed",
                detail_code: Some("shutdown_sequence_panicked".to_string()),
            }],
        }
    }
}

#[derive(Clone, Debug)]
enum ShutdownState {
    Running,
    ShuttingDown,
    Complete(ShutdownReport),
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeShutdownCoordinator {
    shared: Arc<(Mutex<ShutdownState>, Condvar)>,
}

impl Default for RuntimeShutdownCoordinator {
    fn default() -> Self {
        Self {
            shared: Arc::new((Mutex::new(ShutdownState::Running), Condvar::new())),
        }
    }
}

impl RuntimeShutdownCoordinator {
    pub(crate) fn run_once(&self, shutdown: impl FnOnce() -> ShutdownReport) -> ShutdownReport {
        let (state, complete) = &*self.shared;
        let mut guard = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*guard {
            ShutdownState::Running => *guard = ShutdownState::ShuttingDown,
            ShutdownState::Complete(report) => return report.clone(),
            ShutdownState::ShuttingDown => {
                let (waited, _) = complete
                    .wait_timeout_while(guard, SHUTDOWN_OBSERVER_WAIT, |state| {
                        matches!(state, ShutdownState::ShuttingDown)
                    })
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                return match &*waited {
                    ShutdownState::Complete(report) => report.clone(),
                    _ => ShutdownReport {
                        completed: false,
                        phases: vec![ShutdownPhaseReceipt {
                            phase: "shutdown_coordinator",
                            outcome: "failed",
                            detail_code: Some("shutdown_observer_timeout".to_string()),
                        }],
                    },
                };
            }
        }
        drop(guard);

        let mut report =
            catch_unwind(AssertUnwindSafe(shutdown)).unwrap_or_else(|_| ShutdownReport::panicked());
        report.completed = true;
        let mut guard = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = ShutdownState::Complete(report.clone());
        complete.notify_all();
        report
    }
}

pub(crate) fn shutdown_runtime_services_once(
    coordinator: &RuntimeShutdownCoordinator,
    shutdown_service: &crate::gemma::GemmaService,
    app_handle: &tauri::AppHandle,
) -> ShutdownReport {
    let report = coordinator.run_once(|| {
        let mut report = ShutdownReport::default();
        report.run_phase("gateway_intake_and_workers_stopped", || {
            app_handle
                .state::<crate::gateway::SovereignGatewayService>()
                .shutdown_workers();
            Ok(())
        });
        report.run_phase("native_inference_cancelled", || {
            shutdown_service
                .force_shutdown_native_model()
                .map_err(|error| error.code.to_string())
        });
        report.run_phase("local_inference_worker_joined", || {
            crate::inference::shutdown_local_inference_worker();
            Ok(())
        });
        report.run_phase("workflow_scheduler_joined", || {
            app_handle
                .try_state::<crate::workflow_scheduler::WorkflowSchedulerRuntime>()
                .map_or(Ok(()), |runtime| runtime.shutdown())
        });
        report.run_phase("background_runtime_worker_joined", || {
            crate::background_runtime_lifecycle::stop_background_runtime(app_handle)
        });
        report.run_phase("background_hook_workers_joined", || {
            app_handle
                .state::<crate::background_tasks::hooks::BackgroundHookRegistry>()
                .clear_active_mod_hooks();
            Ok(())
        });
        report.run_phase("mcp_children_joined", || {
            app_handle
                .state::<crate::mcp::client::McpClientRegistry>()
                .shutdown_all_blocking()
        });
        report.run_phase("voice_capture_stopped", || {
            app_handle
                .state::<crate::mac_speech::VoiceCaptureManager>()
                .shutdown();
            Ok(())
        });
        report.run_phase("single_instance_listener_joined", || {
            crate::launch_startup::shutdown_single_instance_activation_listener(app_handle)
        });
        report.run_phase("background_ui_removed", || {
            crate::background_runtime_lifecycle::remove_background_ui(app_handle)
        });
        report.run_phase("persistence_flushed", || {
            app_handle
                .state::<crate::db::PersistenceEngine>()
                .open_connection()
                .map_err(|error| error.to_string())?
                .execute_batch("PRAGMA wal_checkpoint(FULL);")
                .map_err(|error| error.to_string())
        });
        report.run_phase("sensitive_session_material_cleared", || {
            app_handle
                .state::<crate::sovereign_identity::SovereignIdentity>()
                .clear_sensitive_session_material();
            crate::keychain_session::clear();
            crate::db::close_all_sqlcipher_sessions(app_handle);
            Ok(())
        });
        report
    });
    for phase in &report.phases {
        eprintln!(
            "OOMU_SHUTDOWN_PHASE phase={} outcome={} detail_code={}",
            phase.phase,
            phase.outcome,
            phase.detail_code.as_deref().unwrap_or("none")
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::{requires_runtime_shutdown, RuntimeShutdownCoordinator, ShutdownReport};
    use std::sync::{Arc, Mutex};

    #[test]
    fn auxiliary_webview_destruction_never_clears_runtime_security_state() {
        assert!(requires_runtime_shutdown("main"));
        for auxiliary in ["sovereign-search", "browser-proxy", "artifact-preview"] {
            assert!(!requires_runtime_shutdown(auxiliary));
        }
    }

    #[test]
    fn sprint_304_repeated_shutdown_observes_one_ordered_sequence() {
        let coordinator = RuntimeShutdownCoordinator::default();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let first_calls = Arc::clone(&calls);
        let first = coordinator.run_once(|| {
            let mut report = ShutdownReport::default();
            for phase in ["stop_intake", "cancel_work", "join_workers", "flush"] {
                let calls = Arc::clone(&first_calls);
                report.run_phase(phase, move || {
                    calls.lock().unwrap().push(phase);
                    Ok(())
                });
            }
            report
        });
        let second = coordinator.run_once(|| panic!("shutdown must not run twice"));

        assert!(first.completed);
        assert_eq!(second, first);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["stop_intake", "cancel_work", "join_workers", "flush"]
        );
    }

    #[test]
    fn sprint_304_shutdown_continues_after_a_phase_failure() {
        let mut report = ShutdownReport::default();
        let calls = Mutex::new(Vec::new());
        report.run_phase("cancel_work", || {
            calls.lock().unwrap().push("cancel_work");
            Err("cancel_failed".to_string())
        });
        report.run_phase("flush", || {
            calls.lock().unwrap().push("flush");
            Ok(())
        });

        assert_eq!(calls.into_inner().unwrap(), ["cancel_work", "flush"]);
        assert_eq!(
            report.phases[0].detail_code.as_deref(),
            Some("cancel_failed")
        );
        assert_eq!(report.phases[1].outcome, "completed");
    }
}
