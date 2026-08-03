use super::*;

impl GemmaService {
    pub(super) fn shutdown_classifier_lane(&self) -> Option<GemmaError> {
        // Every native classifier worker must join before AppKit releases Metal.
        let lane = self
            .classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let error = lane
            .as_ref()
            .and_then(|lane| lane.force_shutdown_native_model().err());
        drop(lane);
        error
    }

    pub(crate) fn verify_classifier_readiness_sync(&self) -> Result<u64, GemmaError> {
        self.run_classifier_readiness_probe()?;
        Ok(self.mark_classifier_ready(classifier_protocol::CLASSIFIER_VERSION))
    }

    pub(crate) fn verify_classifier_readiness_for_recovery_sync(
        &self,
        recovery_epoch: u64,
    ) -> Result<u64, GemmaError> {
        self.run_classifier_readiness_probe()?;
        self.mark_classifier_ready_for_recovery(
            recovery_epoch,
            classifier_protocol::CLASSIFIER_VERSION,
        )
        .ok_or_else(classifier_recovery_superseded)
    }

    fn run_classifier_readiness_probe(&self) -> Result<(), GemmaError> {
        let response = self.infer_classifier_sync(classifier_protocol::request("Say hello."))?;
        classifier_protocol::validated_code(&response.text).map_err(|_| GemmaError {
            code: "classifier_probe_schema_invalid",
            message:
                "The Auto-route classifier readiness probe returned output outside the routing grammar."
                    .to_string(),
        })?;
        Ok(())
    }

    pub(super) fn load_classifier_model_assignment(
        &self,
        assignment: &StartupModelAssignment,
    ) -> Result<(), GemmaError> {
        let lane = Self::prepare_classifier_lane(assignment)?;
        self.commit_classifier_lane(assignment, lane, None)
    }

    pub(super) fn reload_classifier_model_assignment_for_recovery(
        &self,
        recovery_epoch: u64,
    ) -> Result<(), GemmaError> {
        let assignment = {
            let state = self.lock_state();
            if state.classifier_recovery_epoch != recovery_epoch
                || state.classifier_health.status != AutoRouteClassifierStatus::Recovering
            {
                return Err(classifier_recovery_superseded());
            }
            state.startup_assignment.clone().ok_or_else(|| GemmaError {
                code: "classifier_model_not_configured",
                message: "No local classifier model has been configured for this launch."
                    .to_string(),
            })?
        };
        let lane = Self::prepare_classifier_lane(&assignment)?;
        self.commit_classifier_lane(&assignment, lane, Some(recovery_epoch))
    }

    pub(crate) fn reconfigure_startup_model_assignment_for_recovery(
        &self,
        assignment: StartupModelAssignment,
        recovery_epoch: u64,
    ) -> Result<(), GemmaError> {
        let lane = Self::prepare_classifier_lane(&assignment)?;
        {
            let state = self.lock_state();
            if state.classifier_recovery_epoch != recovery_epoch
                || state.classifier_health.status != AutoRouteClassifierStatus::Recovering
            {
                return Err(classifier_recovery_superseded());
            }
        }
        self.commit_classifier_lane(&assignment, lane, Some(recovery_epoch))?;
        let mut state = self.lock_state();
        if state.classifier_recovery_epoch != recovery_epoch
            || state.classifier_health.status != AutoRouteClassifierStatus::Recovering
        {
            return Err(classifier_recovery_superseded());
        }
        state.startup_assignment = Some(assignment.clone());
        state.classifier_health.requested_model_id = Some(assignment.requested_model_id);
        state.classifier_health.classifier_model_id = Some(assignment.resolved_model_id);
        state.classifier_health.selection_source = Some(assignment.selection_source);
        Ok(())
    }

    pub(super) fn classifier_lane_for_model(&self, model_id: &str) -> Option<GemmaService> {
        let requested = model_id.trim();
        let assignment_matches =
            self.lock_state()
                .startup_assignment
                .as_ref()
                .is_some_and(|assignment| {
                    requested.eq_ignore_ascii_case(&assignment.resolved_model_id)
                        || requested.eq_ignore_ascii_case(assignment.identity.canonical_id())
                });
        if !assignment_matches {
            return None;
        }
        self.classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn classifier_lane_if_main_is_empty(&self) -> Option<GemmaService> {
        if self.lock_state().model.is_some() {
            return None;
        }
        self.classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn prepare_classifier_lane(
        assignment: &StartupModelAssignment,
    ) -> Result<GemmaService, GemmaError> {
        let lane = GemmaService::new_loading();
        lane.load_model_from_dir(assignment.resolved_directory.clone())?;
        Ok(lane)
    }

    fn commit_classifier_lane(
        &self,
        assignment: &StartupModelAssignment,
        lane: GemmaService,
        recovery_epoch: Option<u64>,
    ) -> Result<(), GemmaError> {
        let mut classifier_lane = self
            .classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut state = self.lock_state();
        if recovery_epoch.is_some_and(|epoch| {
            state.classifier_recovery_epoch != epoch
                || state.classifier_health.status != AutoRouteClassifierStatus::Recovering
        }) {
            return Err(classifier_recovery_superseded());
        }
        *classifier_lane = Some(lane);
        state.classifier_health.residency_generation = state
            .classifier_health
            .residency_generation
            .saturating_add(1);
        state.classifier_health.verified_residency_generation = 0;
        if state.classifier_health.status != AutoRouteClassifierStatus::Recovering {
            state.classifier_health.status = AutoRouteClassifierStatus::Loading;
        }
        state.classifier_health.classifier_model_id = Some(assignment.resolved_model_id.clone());
        state.classifier_health.last_error_code = None;
        state.classifier_health.last_error_boundary = None;
        state.classifier_health.redacted_recovery_hint = None;
        Ok(())
    }

    pub(crate) fn infer_classifier_sync(
        &self,
        request: InferRequest,
    ) -> Result<InferResponse, GemmaError> {
        let lane = self
            .classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| GemmaError {
                code: "classifier_not_resident",
                message: "The on-device Auto-route model is not ready yet.".to_string(),
            })?;
        lane.infer_sync(request)
    }
}

fn classifier_recovery_superseded() -> GemmaError {
    GemmaError {
        code: "classifier_recovery_superseded",
        message: "A newer classifier lifecycle state superseded this recovery result.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_classifier_recovery_cannot_commit_after_timeout() {
        let service = GemmaService::new_disabled("test runtime intentionally unavailable");
        let recovery_epoch = service.mark_classifier_recovering();
        let before = service.classifier_health().residency_generation;
        service.mark_classifier_failure(
            "classifier_preparation_timeout",
            "auto_route_classifier_preparation",
            "test-owned timeout",
        );
        let assignment = StartupModelAssignment {
            requested_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            resolved_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            resolved_directory: PathBuf::from("/private/tmp/stale-classifier-lane"),
            selection_source: StartupModelSelectionSource::CleanDefault,
            identity: LocalModelIdentity {
                canonical_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
                display_name: "Gemma 4 E2B".to_string(),
                storage_directory: PathBuf::from("/private/tmp/stale-classifier-lane"),
                source: LocalModelIdentitySource::CanonicalRegistry,
            },
        };

        let error = service
            .commit_classifier_lane(
                &assignment,
                GemmaService::new_disabled("prepared test lane"),
                Some(recovery_epoch),
            )
            .expect_err("a timed-out recovery cannot commit its prepared lane");

        assert_eq!(error.code, "classifier_recovery_superseded");
        assert_eq!(service.classifier_health().residency_generation, before);
        assert!(service
            .classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());
    }

    #[test]
    fn stale_failure_cannot_replace_a_newer_classifier_lifecycle() {
        let service = GemmaService::new_disabled("test runtime intentionally unavailable");
        let stale_epoch = service.mark_classifier_recovering();
        let current_epoch = service.mark_classifier_recovering();

        assert!(!service.mark_classifier_failure_for_recovery(
            stale_epoch,
            "classifier_preparation_timeout",
            "auto_route_classifier_preparation",
            "stale failure",
        ));
        assert_eq!(
            service.lock_state().classifier_recovery_epoch,
            current_epoch
        );
        assert_eq!(
            service.classifier_health().status,
            AutoRouteClassifierStatus::Recovering
        );
    }

    #[test]
    fn startup_model_requests_are_owned_only_by_the_classifier_lane() {
        let service = GemmaService::new_disabled("test runtime intentionally unavailable");
        let lane = GemmaService::new_disabled("test lane");
        {
            let mut state = service.lock_state();
            state.startup_assignment = Some(StartupModelAssignment {
                requested_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
                resolved_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
                resolved_directory: PathBuf::from("/private/tmp/classifier-only-e2b"),
                selection_source: StartupModelSelectionSource::CleanDefault,
                identity: LocalModelIdentity {
                    canonical_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
                    display_name: "Gemma 4 E2B".to_string(),
                    storage_directory: PathBuf::from("/private/tmp/classifier-only-e2b"),
                    source: LocalModelIdentitySource::CanonicalRegistry,
                },
            });
        }
        *service
            .classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(lane);

        assert!(service
            .classifier_lane_for_model(CLEAN_INSTALL_STARTUP_MODEL_ID)
            .is_some());
        assert!(service.lock_state().model.is_none());
        assert!(service
            .classifier_lane_for_model("gemma-4-E4B-it")
            .is_none());
    }

    #[test]
    fn application_shutdown_detaches_and_stops_the_classifier_lane() {
        let service = GemmaService::new_disabled("test parent runtime intentionally unavailable");
        let classifier =
            GemmaService::new_disabled("test classifier runtime intentionally unavailable");
        *service
            .classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(classifier.clone());

        service
            .force_shutdown_native_model()
            .expect("shutdown should release every native model lane");

        assert!(service
            .classifier_lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());
        assert_eq!(
            service.classifier_health().status,
            AutoRouteClassifierStatus::Shutdown
        );
        assert_eq!(
            classifier.classifier_health().status,
            AutoRouteClassifierStatus::Shutdown
        );
    }
}
