use super::{
    contracts::{
        AppControlOutcomeStatus, AppControlOutcomeView, AppControlPauseReason, AppControlState,
        DesktopObservation, DesktopOutcomeReceipt, ExecuteDesktopActionRequest,
    },
    error::{AppControlError, AppControlErrorCode, AppControlResult},
    manager::{AppControlManager, ManagerState},
    observation::reference_context,
    policy::{
        validate_expected_outcome, validate_typed_adapter, AuthorityRequest,
        ReviewedScopeDesktopAuthority,
    },
    references::opaque_id,
    state::{
        invalid_request, not_running, pause_session, require_task, session_not_found,
        stale_reference,
    },
    verification::{hash_action_binding, hash_bytes, normalize_action, resolved_file_for_action},
};

impl AppControlManager {
    pub(super) fn pause_for_driver_error(
        &self,
        session_id: &str,
        task_run_id: &str,
        generation: u64,
        error: &AppControlError,
    ) {
        let Ok(mut state) = self.lock() else { return };
        let ManagerState {
            sessions,
            references,
            ..
        } = &mut *state;
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        if session.task_run_id != task_run_id || session.generation != generation {
            return;
        }
        let reason = match error.code {
            AppControlErrorCode::DriverUnavailable => AppControlPauseReason::DriverUnavailable,
            AppControlErrorCode::AccessibilityPermissionMissing
            | AppControlErrorCode::PermissionChanged => AppControlPauseReason::PermissionChanged,
            _ => AppControlPauseReason::AmbiguousTarget,
        };
        pause_session(session, references, self.clock.now_ms(), reason);
    }

    pub(crate) fn authority_request_for(
        &self,
        request: &ExecuteDesktopActionRequest,
    ) -> AppControlResult<AuthorityRequest> {
        let state = self.lock()?;
        let session = state
            .sessions
            .get(&request.session_id)
            .ok_or_else(session_not_found)?;
        require_task(session, &request.task_run_id)?;
        if session.state != AppControlState::Running {
            return Err(not_running());
        }
        let observation = session
            .last_observation
            .as_ref()
            .ok_or_else(|| invalid_request("A fresh app observation is required."))?;
        if observation.revision != request.observation_revision {
            return Err(stale_reference());
        }
        if !observation.window.visible {
            return Err(AppControlError::new(
                AppControlErrorCode::HiddenWindow,
                "The application window is hidden.",
            ));
        }
        if !session
            .approved_bundle_ids
            .contains(&observation.application.bundle_id)
        {
            return Err(AppControlError::new(
                AppControlErrorCode::ApplicationNotAllowlisted,
                "This application is not approved for the current Task.",
            ));
        }
        validate_typed_adapter(&observation.application.bundle_id, &request.action)?;
        validate_expected_outcome(&request.action, request.expected_outcome)?;
        let action = normalize_action(request.action.clone(), &session.file_roots)?;
        if matches!(
            &action,
            super::contracts::DesktopSemanticAction::ChooseFile { .. }
        ) && !observation.window.modal
        {
            return Err(AppControlError::new(
                AppControlErrorCode::AmbiguousTarget,
                "File selection requires a fresh native file dialog.",
            ));
        }
        let selected_file = resolved_file_for_action(&action, &session.file_roots)?;
        let context = reference_context(session, observation, self.clock.now_ms());
        for reference in action.references() {
            let target = state.references.resolve(
                reference,
                action.kind(),
                observation.window.modal,
                &context,
            )?;
            if matches!(
                &action,
                super::contracts::DesktopSemanticAction::ChooseFile { .. }
            ) && !target.in_modal
            {
                return Err(AppControlError::new(
                    AppControlErrorCode::AmbiguousTarget,
                    "The selected target is outside the active file dialog.",
                ));
            }
        }
        Ok(AuthorityRequest {
            project_id: session.project_id.clone(),
            task_run_id: session.task_run_id.clone(),
            session_id: session.session_id.clone(),
            bundle_id: observation.application.bundle_id.clone(),
            action_kind: action.kind(),
            action_arguments_hash: hash_action_binding(&action, selected_file.as_deref())?,
            will_change_data: action.will_change_data(),
        })
    }

    pub(crate) fn register_direct_approval(
        &self,
        authority: &AuthorityRequest,
    ) -> AppControlResult<()> {
        self.authority.register_direct_approval(authority)
    }

    pub(crate) fn approval_binding(
        authority: &AuthorityRequest,
    ) -> super::policy::DesktopApprovalBinding {
        ReviewedScopeDesktopAuthority::approval_binding(authority)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_action_failure(
        &self,
        session_id: &str,
        task_run_id: &str,
        generation: u64,
        before: &DesktopObservation,
        authority: &AuthorityRequest,
        decision_id: &str,
        error: &AppControlError,
    ) {
        let Ok(mut state) = self.lock() else { return };
        let now = self.clock.now_ms();
        let Some(session) = state.sessions.get_mut(session_id) else {
            return;
        };
        if session.task_run_id != task_run_id || session.generation != generation {
            return;
        }
        let receipt = DesktopOutcomeReceipt {
            receipt_id: opaque_id("appreceipt"),
            session_id: session_id.to_string(),
            project_id: before.project_id.clone(),
            task_run_id: task_run_id.to_string(),
            action_kind: authority.action_kind,
            authority_decision_id: decision_id.to_string(),
            before_observation_id: before.observation_id.clone(),
            before_observation_hash: before.observation_hash.clone(),
            after_observation_id: before.observation_id.clone(),
            after_observation_hash: before.observation_hash.clone(),
            action_arguments_hash: authority.action_arguments_hash.clone(),
            driver_receipt_hash: hash_bytes(format!("{:?}", error.code).as_bytes()),
            postcondition_hash: hash_bytes(error.message.as_bytes()),
            file_hashes: Vec::new(),
            status: AppControlOutcomeStatus::Failed,
            recorded_at_ms: now,
        };
        session.current_action = None;
        session.last_outcome = Some(AppControlOutcomeView {
            status: AppControlOutcomeStatus::Failed,
            action_kind: authority.action_kind,
            receipt_id: receipt.receipt_id.clone(),
            recorded_at_ms: now,
            details_available: true,
        });
        session.updated_at_ms = now;
        state
            .receipts
            .insert(receipt.receipt_id.clone(), receipt.clone());
        drop(state);
        let _ = self.record_task_event(
            task_run_id,
            "app_control.action_receipt",
            crate::p0_contracts::EvidenceClass::ObservedResult,
            serde_json::json!({ "receipt": receipt, "failureCode": error.code }),
        );
    }
}
