use super::{
    contracts::{
        AppControlActionView, AppControlOutcomeStatus, AppControlOutcomeView,
        AppControlPauseReason, AppControlState, DesktopActionOutcome, DesktopObservation,
        DesktopOutcomeReceipt, ExecuteDesktopActionRequest, ExpectedOutcomeKind,
    },
    driver::{
        DesktopDriver, DriverActionRequest, DriverCancellationToken, UnavailableDesktopDriver,
    },
    error::{AppControlError, AppControlErrorCode, AppControlResult},
    observation::{
        apply_observation, driver_observation_request, driver_observation_request_from,
        reference_context,
    },
    policy::{
        validate_expected_outcome, validate_typed_adapter, AuthorityRequest,
        DenyAllDesktopAuthority, DesktopAuthorityEvaluator, ScopedFileRoots,
    },
    references::{opaque_id, ReferenceVault},
    state::{
        ensure_generation, invalid_request, invalidate_generation, not_running, pause_session,
        require_task, session_not_found, session_view, stale_reference,
    },
    verification::{
        hash_action_binding, hash_bytes, hash_serializable, normalize_action,
        resolved_file_for_action, verify_postcondition,
    },
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) const TERMINAL_STATUS_GRACE_MS: i64 = 15_000;
pub(super) const FILE_GRANT_TTL_MS: i64 = 10 * 60 * 1_000;

#[derive(Clone)]
pub(super) struct PendingFileGrant {
    pub(super) project_id: String,
    pub(super) task_run_id: String,
    pub(super) canonical_file: PathBuf,
    pub(super) expires_at_ms: i64,
}

pub trait AppControlTimeSource: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Default)]
pub struct SystemAppControlTimeSource;

impl AppControlTimeSource for SystemAppControlTimeSource {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or_default()
    }
}

pub(super) struct SessionRecord {
    pub(super) session_id: String,
    pub(super) project_id: String,
    pub(super) task_run_id: String,
    pub(super) approved_bundle_ids: HashSet<String>,
    pub(super) file_roots: ScopedFileRoots,
    pub(super) state: AppControlState,
    pub(super) generation: u64,
    pub(super) revision: u64,
    pub(super) current_action: Option<AppControlActionView>,
    pub(super) pause_reason: Option<AppControlPauseReason>,
    pub(super) last_outcome: Option<AppControlOutcomeView>,
    pub(super) last_observation: Option<DesktopObservation>,
    pub(super) updated_at_ms: i64,
    pub(super) cancellation_epoch: Arc<AtomicU64>,
    pub(super) mismatch_count: u8,
}

#[derive(Default)]
pub(super) struct ManagerState {
    pub(super) sessions: HashMap<String, SessionRecord>,
    pub(super) references: ReferenceVault,
    pub(super) receipts: HashMap<String, DesktopOutcomeReceipt>,
    pub(super) active_session_id: Option<String>,
    pub(super) pending_file_grants: HashMap<String, PendingFileGrant>,
}

pub struct AppControlManager {
    pub(super) driver: Arc<dyn DesktopDriver>,
    pub(super) authority: Arc<dyn DesktopAuthorityEvaluator>,
    pub(super) clock: Arc<dyn AppControlTimeSource>,
    pub(super) evidence_engine: Option<crate::db::PersistenceEngine>,
    physical_input_epoch: Option<Arc<AtomicU64>>,
    physical_input_ready: Option<Arc<AtomicBool>>,
    acknowledged_input_epoch: AtomicU64,
    pub(super) inner: Mutex<ManagerState>,
}

impl Default for AppControlManager {
    fn default() -> Self {
        Self::new(
            Arc::new(UnavailableDesktopDriver),
            Arc::new(DenyAllDesktopAuthority),
            Arc::new(SystemAppControlTimeSource),
        )
    }
}

impl AppControlManager {
    pub fn new(
        driver: Arc<dyn DesktopDriver>,
        authority: Arc<dyn DesktopAuthorityEvaluator>,
        clock: Arc<dyn AppControlTimeSource>,
    ) -> Self {
        Self {
            driver,
            authority,
            clock,
            evidence_engine: None,
            physical_input_epoch: None,
            physical_input_ready: None,
            acknowledged_input_epoch: AtomicU64::new(0),
            inner: Mutex::new(ManagerState::default()),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn production(engine: crate::db::PersistenceEngine) -> Self {
        let physical_input = super::macos_input::install_physical_input_monitor();
        let acknowledged_input_epoch = physical_input.epoch.load(Ordering::SeqCst);
        Self {
            driver: Arc::new(super::macos_driver::MacOsAccessibilityDriver::default()),
            authority: Arc::new(super::policy::ReviewedScopeDesktopAuthority::new(
                engine.clone(),
            )),
            clock: Arc::new(SystemAppControlTimeSource),
            evidence_engine: Some(engine),
            physical_input_epoch: Some(physical_input.epoch),
            physical_input_ready: Some(physical_input.ready),
            acknowledged_input_epoch: AtomicU64::new(acknowledged_input_epoch),
            inner: Mutex::new(ManagerState::default()),
        }
    }

    pub fn observe(
        &self,
        session_id: &str,
        task_run_id: &str,
    ) -> AppControlResult<DesktopObservation> {
        let (request, generation) = {
            let state = self.lock()?;
            let session = state
                .sessions
                .get(session_id)
                .ok_or_else(session_not_found)?;
            require_task(session, task_run_id)?;
            if !session.state.active() || session.state == AppControlState::Takeover {
                return Err(not_running());
            }
            (driver_observation_request(session), session.generation)
        };

        let raw = self.driver.observe(&request).map_err(|error| {
            self.pause_for_driver_error(session_id, task_run_id, generation, &error);
            error
        })?;
        let mut state = self.lock()?;
        ensure_generation(&state, session_id, task_run_id, generation, None)?;
        let observation =
            apply_observation(&mut state, session_id, raw, self.clock.now_ms(), false)?;
        drop(state);
        self.record_observation(&observation, "before_or_resume")?;
        Ok(observation)
    }

    pub fn execute(
        &self,
        request: ExecuteDesktopActionRequest,
    ) -> AppControlResult<DesktopActionOutcome> {
        let physical_input_binding = self.physical_input_binding();
        let now = self.clock.now_ms();
        let (before, action, targets, selected_file, target_label, authority_request, generation) = {
            let mut state = self.lock()?;
            let ManagerState {
                sessions,
                references,
                ..
            } = &mut *state;
            let session = sessions
                .get_mut(&request.session_id)
                .ok_or_else(session_not_found)?;
            require_task(session, &request.task_run_id)?;
            if session.state != AppControlState::Running {
                return Err(not_running());
            }
            let before = session
                .last_observation
                .clone()
                .ok_or_else(|| invalid_request("A fresh app observation is required."))?;
            if before.revision != request.observation_revision {
                return Err(stale_reference());
            }
            if !before.window.visible {
                pause_session(
                    session,
                    references,
                    now,
                    AppControlPauseReason::HiddenWindow,
                );
                return Err(AppControlError::new(
                    AppControlErrorCode::HiddenWindow,
                    "The application window is hidden.",
                ));
            }
            if !session
                .approved_bundle_ids
                .contains(&before.application.bundle_id)
            {
                return Err(AppControlError::new(
                    AppControlErrorCode::ApplicationNotAllowlisted,
                    "This application is not approved for the current Task.",
                ));
            }
            validate_typed_adapter(&before.application.bundle_id, &request.action)?;
            validate_expected_outcome(&request.action, request.expected_outcome)?;
            let action = normalize_action(request.action, &session.file_roots)?;
            if matches!(
                &action,
                super::contracts::DesktopSemanticAction::ChooseFile { .. }
            ) && !before.window.modal
            {
                return Err(AppControlError::new(
                    AppControlErrorCode::AmbiguousTarget,
                    "File selection requires a fresh native file dialog.",
                ));
            }
            let selected_file = resolved_file_for_action(&action, &session.file_roots)?;
            let target_label = match &action {
                super::contracts::DesktopSemanticAction::ChooseFile { file_grant_id, .. } => {
                    session.file_roots.file_name(file_grant_id)
                }
                _ => action.target_label(),
            };
            let action_hash = hash_action_binding(&action, selected_file.as_deref())?;
            let context = reference_context(session, &before, now);
            let mut targets = Vec::new();
            for reference in action.references() {
                targets.push(references.resolve(
                    reference,
                    action.kind(),
                    before.window.modal,
                    &context,
                )?);
            }
            if matches!(
                &action,
                super::contracts::DesktopSemanticAction::ChooseFile { .. }
            ) && targets.iter().any(|target| !target.in_modal)
            {
                return Err(AppControlError::new(
                    AppControlErrorCode::AmbiguousTarget,
                    "The selected target is outside the active file dialog.",
                ));
            }
            let authority_request = AuthorityRequest {
                project_id: session.project_id.clone(),
                task_run_id: session.task_run_id.clone(),
                session_id: session.session_id.clone(),
                bundle_id: before.application.bundle_id.clone(),
                action_kind: action.kind(),
                action_arguments_hash: action_hash,
                will_change_data: action.will_change_data(),
            };
            (
                before,
                action,
                targets,
                selected_file,
                target_label,
                authority_request,
                session.generation,
            )
        };

        let decision = self.authority.evaluate(&authority_request)?;
        if !decision.authorized || decision.decision_id.trim().is_empty() {
            return Err(AppControlError::new(
                AppControlErrorCode::Unauthorized,
                "The Task did not authorize this app action.",
            ));
        }

        let driver_request = {
            let mut state = self.lock()?;
            ensure_generation(
                &state,
                &request.session_id,
                &request.task_run_id,
                generation,
                Some(before.revision),
            )?;
            let session = state
                .sessions
                .get_mut(&request.session_id)
                .ok_or_else(session_not_found)?;
            if session.state != AppControlState::Running {
                return Err(not_running());
            }
            session.current_action = Some(AppControlActionView {
                kind: action.kind(),
                target_label,
                will_change_data: action.will_change_data(),
            });
            session.updated_at_ms = self.clock.now_ms();
            DriverActionRequest {
                session_id: session.session_id.clone(),
                project_id: session.project_id.clone(),
                task_run_id: session.task_run_id.clone(),
                application: before.application.clone(),
                window: before.window.clone(),
                action: action.clone(),
                expected_outcome: request.expected_outcome,
                targets: targets.clone(),
                selected_file: selected_file.clone(),
                cancellation: DriverCancellationToken::new(
                    Arc::clone(&session.cancellation_epoch),
                    session.cancellation_epoch.load(Ordering::SeqCst),
                    physical_input_binding,
                ),
            }
        };

        let driver_result = self.driver.perform(&driver_request).map_err(|error| {
            self.record_action_failure(
                &request.session_id,
                &request.task_run_id,
                generation,
                &before,
                &authority_request,
                &decision.decision_id,
                &error,
            );
            error
        })?;

        {
            let state = self.lock()?;
            ensure_generation(
                &state,
                &request.session_id,
                &request.task_run_id,
                generation,
                Some(before.revision),
            )?;
        }
        let after_raw = self
            .driver
            .observe(&driver_observation_request_from(&driver_request))
            .map_err(|error| {
                self.record_action_failure(
                    &request.session_id,
                    &request.task_run_id,
                    generation,
                    &before,
                    &authority_request,
                    &decision.decision_id,
                    &error,
                );
                error
            })?;

        let mut state = self.lock()?;
        ensure_generation(
            &state,
            &request.session_id,
            &request.task_run_id,
            generation,
            Some(before.revision),
        )?;
        let after = apply_observation(
            &mut state,
            &request.session_id,
            after_raw,
            self.clock.now_ms(),
            request.expected_outcome == ExpectedOutcomeKind::WindowState,
        )?;

        let verification = verify_postcondition(
            &driver_result.postcondition,
            request.expected_outcome,
            &action,
            &targets,
            &before,
            &after,
            &state
                .sessions
                .get(&request.session_id)
                .ok_or_else(session_not_found)?
                .file_roots,
        );
        let file_hashes = match verification {
            Ok(file_hashes) => file_hashes,
            Err(error) => {
                let ManagerState {
                    sessions,
                    references,
                    ..
                } = &mut *state;
                if let Some(session) = sessions.get_mut(&request.session_id) {
                    session.current_action = None;
                    session.mismatch_count = session.mismatch_count.saturating_add(1);
                    if session.mismatch_count >= 2 {
                        pause_session(
                            session,
                            references,
                            self.clock.now_ms(),
                            AppControlPauseReason::RepeatedMismatch,
                        );
                    }
                }
                return Err(error);
            }
        };

        let status = if request.expected_outcome == ExpectedOutcomeKind::NoChange {
            AppControlOutcomeStatus::NoChange
        } else {
            AppControlOutcomeStatus::Verified
        };
        let recorded_at_ms = self.clock.now_ms();
        let receipt = DesktopOutcomeReceipt {
            receipt_id: opaque_id("appreceipt"),
            session_id: request.session_id.clone(),
            project_id: before.project_id.clone(),
            task_run_id: request.task_run_id.clone(),
            action_kind: action.kind(),
            authority_decision_id: decision.decision_id,
            before_observation_id: before.observation_id.clone(),
            before_observation_hash: before.observation_hash.clone(),
            after_observation_id: after.observation_id.clone(),
            after_observation_hash: after.observation_hash.clone(),
            action_arguments_hash: authority_request.action_arguments_hash,
            driver_receipt_hash: hash_bytes(driver_result.receipt_token.as_bytes()),
            postcondition_hash: hash_serializable(&driver_result.postcondition)?,
            file_hashes,
            status,
            recorded_at_ms,
        };
        let session = state
            .sessions
            .get_mut(&request.session_id)
            .ok_or_else(session_not_found)?;
        if session.state != AppControlState::Running {
            return Err(AppControlError::new(
                AppControlErrorCode::UnexpectedNavigation,
                "The application changed while verifying the action.",
            ));
        }
        session.current_action = None;
        session.mismatch_count = 0;
        session.last_outcome = Some(AppControlOutcomeView {
            status,
            action_kind: action.kind(),
            receipt_id: receipt.receipt_id.clone(),
            recorded_at_ms,
            details_available: true,
        });
        session.updated_at_ms = recorded_at_ms;
        let view = session_view(session);
        state
            .receipts
            .insert(receipt.receipt_id.clone(), receipt.clone());
        let outcome = DesktopActionOutcome {
            receipt,
            observation: after,
            session: view,
        };
        drop(state);
        self.record_observation(&outcome.observation, "after_action")?;
        self.record_task_event(
            &outcome.receipt.task_run_id,
            "app_control.action_receipt",
            crate::p0_contracts::EvidenceClass::VerifiedPostcondition,
            serde_json::json!({ "receipt": &outcome.receipt }),
        )?;
        Ok(outcome)
    }

    pub(super) fn lock(&self) -> AppControlResult<MutexGuard<'_, ManagerState>> {
        let mut state = self.inner.lock().map_err(|_| {
            AppControlError::new(
                AppControlErrorCode::DriverFailure,
                "App control state is unavailable.",
            )
        })?;
        if let Some(ready) = &self.physical_input_ready {
            let now = self.clock.now_ms();
            let ManagerState {
                sessions,
                references,
                ..
            } = &mut *state;
            if !ready.load(Ordering::SeqCst) {
                for session in sessions.values_mut().filter(|session| {
                    session.state.active()
                        && !(session.state == AppControlState::Paused
                            && session.pause_reason
                                == Some(AppControlPauseReason::DriverUnavailable))
                }) {
                    pause_session(
                        session,
                        references,
                        now,
                        AppControlPauseReason::DriverUnavailable,
                    );
                }
            } else {
                for session in sessions.values_mut().filter(|session| {
                    session.state == AppControlState::Paused
                        && session.pause_reason == Some(AppControlPauseReason::DriverUnavailable)
                }) {
                    invalidate_generation(session, references, now);
                    session.state = AppControlState::ReturnPending;
                    session.pause_reason = None;
                }
            }
        }
        if let Some(epoch) = &self.physical_input_epoch {
            let current = epoch.load(Ordering::SeqCst);
            let previous = self
                .acknowledged_input_epoch
                .swap(current, Ordering::SeqCst);
            if current != previous {
                let now = self.clock.now_ms();
                let ManagerState {
                    sessions,
                    references,
                    ..
                } = &mut *state;
                for session in sessions.values_mut().filter(|session| {
                    session.state.active() && session.state != AppControlState::Takeover
                }) {
                    pause_session(session, references, now, AppControlPauseReason::UserInput);
                }
            }
        }
        Ok(state)
    }

    #[cfg(test)]
    pub(super) fn attach_test_input_monitor(
        mut self,
        epoch: Arc<AtomicU64>,
        ready: Arc<AtomicBool>,
    ) -> Self {
        self.acknowledged_input_epoch = AtomicU64::new(epoch.load(Ordering::SeqCst));
        self.physical_input_epoch = Some(epoch);
        self.physical_input_ready = Some(ready);
        self
    }

    #[cfg(test)]
    pub(super) fn attach_test_evidence_engine(
        mut self,
        engine: crate::db::PersistenceEngine,
    ) -> Self {
        self.evidence_engine = Some(engine);
        self
    }

    pub(super) fn physical_input_binding(&self) -> Option<(Arc<AtomicU64>, u64)> {
        self.physical_input_epoch
            .as_ref()
            .map(|epoch| (Arc::clone(epoch), epoch.load(Ordering::SeqCst)))
    }
}
