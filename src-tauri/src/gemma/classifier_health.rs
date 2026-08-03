use super::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoRouteClassifierStatus {
    Loading,
    Ready,
    Recovering,
    Degraded,
    Shutdown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoRouteClassifierHealth {
    pub status: AutoRouteClassifierStatus,
    pub requested_model_id: Option<String>,
    pub classifier_model_id: Option<String>,
    pub selection_source: Option<StartupModelSelectionSource>,
    pub classifier_version: String,
    pub readiness_generation: u64,
    pub residency_generation: u64,
    pub verified_residency_generation: u64,
    pub last_verified_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
    pub last_error_boundary: Option<String>,
    pub redacted_recovery_hint: Option<String>,
}

impl AutoRouteClassifierHealth {
    pub(super) fn loading() -> Self {
        Self {
            status: AutoRouteClassifierStatus::Loading,
            requested_model_id: None,
            classifier_model_id: None,
            selection_source: None,
            classifier_version: classifier_protocol::CLASSIFIER_VERSION.to_string(),
            readiness_generation: 0,
            residency_generation: 0,
            verified_residency_generation: 0,
            last_verified_at_ms: None,
            last_error_code: None,
            last_error_boundary: None,
            redacted_recovery_hint: None,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.status == AutoRouteClassifierStatus::Ready
            && self.last_verified_at_ms.is_some()
            && self.classifier_model_id.is_some()
            && self.requested_model_id.is_some()
            && self.residency_generation > 0
            && self.residency_generation == self.verified_residency_generation
    }

    pub fn matches_startup_assignment(&self, assignment: &StartupModelAssignment) -> bool {
        self.requested_model_id.as_deref() == Some(assignment.requested_model_id.as_str())
            && self.classifier_model_id.as_deref() == Some(assignment.resolved_model_id.as_str())
            && self.selection_source == Some(assignment.selection_source)
    }
}

impl GemmaService {
    pub fn new_loading() -> Self {
        match catch_unwind(AssertUnwindSafe(NativeRuntime::initialize)) {
            Ok(Ok(runtime)) => {
                eprintln!(
                    "LLAMA_CPP_RUNTIME_READY os={} arch={} apple_silicon={} metal={} accelerator={} gpu_offload={} threads={} memory_bytes={} context_size={} gpu_layers={}",
                    runtime.hardware().operating_system,
                    runtime.hardware().architecture,
                    runtime.hardware().apple_silicon,
                    runtime.hardware().metal_available,
                    runtime.hardware().accelerator_name.as_deref().unwrap_or("none"),
                    runtime.hardware().gpu_offload_available,
                    runtime.hardware().logical_threads,
                    runtime.hardware().total_memory_bytes,
                    runtime.config().context_size,
                    runtime.config().requested_gpu_layers,
                );
                Self {
                    state: Arc::new(Mutex::new(GemmaServiceState {
                        status: GemmaStatus::Loading,
                        model: None,
                        startup_assignment: None,
                        keep_resident: true,
                        degraded_reason: None,
                        classifier_health: AutoRouteClassifierHealth::loading(),
                        classifier_recovery_epoch: 0,
                    })),
                    model_load: Arc::new(Mutex::new(())),
                    runtime: Some(runtime),
                    audit_persistence: Arc::new(Mutex::new(None)),
                    classifier_lane: Arc::new(Mutex::new(None)),
                }
            }
            Ok(Err(error)) => Self::new_disabled(error.message),
            Err(_) => Self::new_disabled(
                "llama.cpp backend initialization panicked and was safely contained.",
            ),
        }
    }

    pub fn new_disabled(reason: impl Into<String>) -> Self {
        Self {
            state: Arc::new(Mutex::new(GemmaServiceState {
                status: GemmaStatus::Degraded,
                model: None,
                startup_assignment: None,
                keep_resident: true,
                degraded_reason: Some(reason.into()),
                classifier_health: AutoRouteClassifierHealth {
                    status: AutoRouteClassifierStatus::Degraded,
                    last_error_code: Some("classifier_runtime_unavailable".to_string()),
                    last_error_boundary: Some("native_runtime_initialization".to_string()),
                    redacted_recovery_hint: Some(
                        "Restart OOMU after the local inference runtime is available.".to_string(),
                    ),
                    ..AutoRouteClassifierHealth::loading()
                },
                classifier_recovery_epoch: 0,
            })),
            model_load: Arc::new(Mutex::new(())),
            runtime: None,
            audit_persistence: Arc::new(Mutex::new(None)),
            classifier_lane: Arc::new(Mutex::new(None)),
        }
    }

    pub fn attach_audit_persistence(&self, persistence: PersistenceEngine) {
        if let Ok(mut attached) = self.audit_persistence.lock() {
            *attached = Some(persistence);
        }
    }

    pub fn classifier_health(&self) -> AutoRouteClassifierHealth {
        self.lock_state().classifier_health.clone()
    }

    pub fn load_startup_model_assignment(
        &self,
        assignment: StartupModelAssignment,
    ) -> Result<(), GemmaError> {
        {
            let mut state = self.lock_state();
            state.startup_assignment = Some(assignment.clone());
            state.classifier_health.requested_model_id =
                Some(assignment.requested_model_id.clone());
            state.classifier_health.classifier_model_id =
                Some(assignment.resolved_model_id.clone());
            state.classifier_health.selection_source = Some(assignment.selection_source);
        }
        self.load_classifier_model_assignment(&assignment)?;
        let mut state = self.lock_state();
        state.classifier_health.requested_model_id = Some(assignment.requested_model_id);
        state.classifier_health.classifier_model_id = Some(assignment.resolved_model_id);
        state.classifier_health.selection_source = Some(assignment.selection_source);
        Ok(())
    }

    pub fn startup_model_assignment(&self) -> Option<StartupModelAssignment> {
        self.lock_state().startup_assignment.clone()
    }

    pub fn mark_classifier_recovering(&self) -> u64 {
        let mut state = self.lock_state();
        state.classifier_recovery_epoch = state.classifier_recovery_epoch.saturating_add(1);
        state.classifier_health.status = AutoRouteClassifierStatus::Recovering;
        state.classifier_health.verified_residency_generation = 0;
        state.classifier_health.last_verified_at_ms = None;
        state.classifier_health.redacted_recovery_hint =
            Some("OOMU is verifying the local Auto-route classifier.".to_string());
        state.classifier_recovery_epoch
    }

    pub fn mark_classifier_ready(&self, classifier_version: &str) -> u64 {
        let (next_generation, health) = {
            let mut state = self.lock_state();
            let next_generation = state
                .classifier_health
                .readiness_generation
                .saturating_add(1);
            state.classifier_health.status = AutoRouteClassifierStatus::Ready;
            state.classifier_health.classifier_version = classifier_version.to_string();
            state.classifier_health.readiness_generation = next_generation;
            state.classifier_health.verified_residency_generation =
                state.classifier_health.residency_generation;
            state.classifier_health.last_verified_at_ms =
                Some(crate::foundation::clock::unix_time_ms_i64());
            state.classifier_health.last_error_code = None;
            state.classifier_health.last_error_boundary = None;
            state.classifier_health.redacted_recovery_hint = None;
            (next_generation, state.classifier_health.clone())
        };
        self.persist_classifier_readiness_event(&health);
        next_generation
    }

    pub fn mark_classifier_ready_for_recovery(
        &self,
        recovery_epoch: u64,
        classifier_version: &str,
    ) -> Option<u64> {
        let (next_generation, health) = {
            let mut state = self.lock_state();
            if state.classifier_recovery_epoch != recovery_epoch
                || state.classifier_health.status != AutoRouteClassifierStatus::Recovering
            {
                return None;
            }
            let next_generation = state
                .classifier_health
                .readiness_generation
                .saturating_add(1);
            state.classifier_health.status = AutoRouteClassifierStatus::Ready;
            state.classifier_health.classifier_version = classifier_version.to_string();
            state.classifier_health.readiness_generation = next_generation;
            state.classifier_health.verified_residency_generation =
                state.classifier_health.residency_generation;
            state.classifier_health.last_verified_at_ms =
                Some(crate::foundation::clock::unix_time_ms_i64());
            state.classifier_health.last_error_code = None;
            state.classifier_health.last_error_boundary = None;
            state.classifier_health.redacted_recovery_hint = None;
            (next_generation, state.classifier_health.clone())
        };
        self.persist_classifier_readiness_event(&health);
        Some(next_generation)
    }

    pub fn mark_classifier_failure(&self, code: &str, boundary: &str, message: &str) {
        {
            let mut state = self.lock_state();
            state.classifier_recovery_epoch = state.classifier_recovery_epoch.saturating_add(1);
            state.classifier_health.status = AutoRouteClassifierStatus::Degraded;
            state.classifier_health.last_error_code = Some(code.to_string());
            state.classifier_health.last_error_boundary = Some(boundary.to_string());
            state.classifier_health.redacted_recovery_hint =
                Some("Retry Auto-route or choose the saved local model for this turn.".to_string());
        }
        self.persist_classifier_health_event(code, boundary, message);
    }

    pub fn mark_classifier_failure_for_recovery(
        &self,
        recovery_epoch: u64,
        code: &str,
        boundary: &str,
        message: &str,
    ) -> bool {
        {
            let mut state = self.lock_state();
            if state.classifier_recovery_epoch != recovery_epoch {
                return false;
            }
            state.classifier_recovery_epoch = state.classifier_recovery_epoch.saturating_add(1);
            state.classifier_health.status = AutoRouteClassifierStatus::Degraded;
            state.classifier_health.verified_residency_generation = 0;
            state.classifier_health.last_verified_at_ms = None;
            state.classifier_health.last_error_code = Some(code.to_string());
            state.classifier_health.last_error_boundary = Some(boundary.to_string());
            state.classifier_health.redacted_recovery_hint =
                Some("Choose an available on-device model, then try again.".to_string());
        }
        self.persist_classifier_health_event(code, boundary, message);
        true
    }

    pub fn reload_configured_classifier_for_recovery(
        &self,
        recovery_epoch: u64,
    ) -> Result<(), GemmaError> {
        self.reload_classifier_model_assignment_for_recovery(recovery_epoch)
    }

    fn persist_classifier_readiness_event(&self, health: &AutoRouteClassifierHealth) {
        let requested = health.requested_model_id.as_deref().unwrap_or("unassigned");
        let resolved = health
            .classifier_model_id
            .as_deref()
            .unwrap_or("unassigned");
        let source = health
            .selection_source
            .map(StartupModelSelectionSource::as_str)
            .unwrap_or("unassigned");
        let event = format!(
            "requested_model_id={requested};resolved_model_id={resolved};selection_source={source};readiness_generation={};residency_generation={}",
            health.readiness_generation,
            health.residency_generation,
        );
        crate::diagnostic_output::write_diagnostic_line(format_args!(
            "OOMU_NATIVE_RECEIPT {}",
            serde_json::json!({
                "kind": "auto_route_classifier_ready",
                "requestedModelId": requested,
                "resolvedModelId": resolved,
                "selectionSource": source,
                "classifierVersion": health.classifier_version,
                "readinessGeneration": health.readiness_generation,
                "residencyGeneration": health.residency_generation,
                "verifiedAtMs": health.last_verified_at_ms,
                "readinessProbe": "grammar_constrained_inference"
            })
        ));
        self.persist_classifier_health_event(
            "classifier_ready",
            "auto_route_classifier_readiness",
            &event,
        );
    }

    pub(super) fn persist_classifier_health_event(
        &self,
        code: &str,
        boundary: &str,
        message: &str,
    ) {
        let persistence = self
            .audit_persistence
            .lock()
            .ok()
            .and_then(|attached| attached.clone());
        let Some(persistence) = persistence else {
            return;
        };
        let redacted_message = crate::redaction::redacted_log_text(message);
        let event = format!("code={code};boundary={boundary}");
        let trace_hash = sha256_hex(
            format!("auto-route-classifier-health:{event}:{redacted_message}").as_bytes(),
        );
        if let Err(error) = persistence.insert_local_inference_audit(
            "auto_route_classifier_health",
            &event,
            &redacted_message,
            &trace_hash,
            "native_classifier_health",
            0,
            0,
            0,
            0,
        ) {
            eprintln!(
                "AUTO_ROUTE_CLASSIFIER_HEALTH_AUDIT_FAILED code={} error={}",
                crate::redaction::redacted_log_text(code),
                crate::redaction::redacted_log_text(&error.to_string())
            );
        }
    }
}

#[cfg(test)]
mod startup_assignment_tests {
    use super::*;

    #[test]
    fn classifier_uses_verified_startup_assignment() {
        let service = GemmaService::new_disabled("test runtime intentionally unavailable");
        let assignment = StartupModelAssignment {
            requested_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            resolved_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            resolved_directory: PathBuf::from("/private/tmp/verified-e2b"),
            selection_source: StartupModelSelectionSource::CleanDefault,
            identity: LocalModelIdentity {
                canonical_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
                display_name: "Gemma 4 E2B".to_string(),
                storage_directory: PathBuf::from("/private/tmp/verified-e2b"),
                source: LocalModelIdentitySource::CanonicalRegistry,
            },
        };
        let _ = service.load_startup_model_assignment(assignment.clone());

        assert_eq!(service.startup_model_assignment(), Some(assignment));
        let health = service.classifier_health();
        assert_eq!(
            health.requested_model_id.as_deref(),
            Some(CLEAN_INSTALL_STARTUP_MODEL_ID)
        );
        assert_eq!(
            health.classifier_model_id.as_deref(),
            Some(CLEAN_INSTALL_STARTUP_MODEL_ID)
        );
        assert_eq!(
            health.selection_source,
            Some(StartupModelSelectionSource::CleanDefault)
        );
    }

    #[test]
    fn auto_route_classifier_residency_is_not_replaced_by_other_local_work() {
        let service = GemmaService::new_disabled("test runtime intentionally unavailable");
        {
            let mut state = service.lock_state();
            state.classifier_health.status = AutoRouteClassifierStatus::Ready;
            state.classifier_health.requested_model_id =
                Some(CLEAN_INSTALL_STARTUP_MODEL_ID.to_string());
            state.classifier_health.classifier_model_id =
                Some(CLEAN_INSTALL_STARTUP_MODEL_ID.to_string());
            state.classifier_health.residency_generation = 7;
            state.classifier_health.verified_residency_generation = 7;
            state.classifier_health.last_verified_at_ms = Some(1);
        }
        let before = service.classifier_health();
        service.enter_local_generation_degraded(GemmaError {
            code: "local_generation_test_failure",
            message: "test-owned local generation failure".to_string(),
        });
        let after = service.classifier_health();

        assert!(after.is_ready());
        assert_eq!(after.classifier_model_id, before.classifier_model_id);
        assert_eq!(after.residency_generation, before.residency_generation);
        assert_eq!(
            after.verified_residency_generation,
            before.verified_residency_generation
        );
    }

    #[test]
    fn ready_health_must_match_the_current_persisted_assignment() {
        let service = GemmaService::new_disabled("test runtime intentionally unavailable");
        let assignment = StartupModelAssignment {
            requested_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            resolved_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            resolved_directory: PathBuf::from("/private/tmp/verified-e2b"),
            selection_source: StartupModelSelectionSource::CleanDefault,
            identity: LocalModelIdentity {
                canonical_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
                display_name: "Gemma 4 E2B".to_string(),
                storage_directory: PathBuf::from("/private/tmp/verified-e2b"),
                source: LocalModelIdentitySource::CanonicalRegistry,
            },
        };
        {
            let mut state = service.lock_state();
            state.classifier_health.status = AutoRouteClassifierStatus::Ready;
            state.classifier_health.requested_model_id =
                Some(assignment.requested_model_id.clone());
            state.classifier_health.classifier_model_id =
                Some(assignment.resolved_model_id.clone());
            state.classifier_health.selection_source = Some(assignment.selection_source);
            state.classifier_health.residency_generation = 3;
            state.classifier_health.verified_residency_generation = 3;
            state.classifier_health.last_verified_at_ms = Some(1);
        }

        assert!(service.classifier_health().is_ready());
        assert!(service
            .classifier_health()
            .matches_startup_assignment(&assignment));
        let changed = StartupModelAssignment {
            requested_model_id: GEMMA_E4B_CANONICAL_ID.to_string(),
            resolved_model_id: GEMMA_E4B_CANONICAL_ID.to_string(),
            selection_source: StartupModelSelectionSource::ExplicitUserSelection,
            ..assignment
        };
        assert!(!service
            .classifier_health()
            .matches_startup_assignment(&changed));
    }
}
