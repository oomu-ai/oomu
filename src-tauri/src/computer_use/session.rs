use super::{
    contracts::{
        AppControlControl, AppControlFileGrantView, AppControlPauseReason, AppControlSessionView,
        AppControlState, ControlAppControlSessionRequest, StartAppControlSession,
    },
    error::{AppControlError, AppControlErrorCode, AppControlResult},
    manager::{
        AppControlManager, ManagerState, PendingFileGrant, SessionRecord, FILE_GRANT_TTL_MS,
        TERMINAL_STATUS_GRACE_MS,
    },
    policy::{app_profile, valid_file_grant_id, ApplicationQualification, ScopedFileRoots},
    references::opaque_id,
    state::{
        invalid_request, invalidate_generation, not_running, require_task, session_not_found,
        session_view, valid_bundle_id,
    },
};
use crate::p0_contracts::{ProjectId, TaskRunId};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

impl AppControlManager {
    pub fn grant_selected_file(
        &self,
        project_id: &str,
        task_run_id: &str,
        selected_file: PathBuf,
    ) -> AppControlResult<AppControlFileGrantView> {
        let project_id = ProjectId::parse(project_id)
            .map_err(invalid_request)?
            .to_string();
        let task_run_id = TaskRunId::parse(task_run_id)
            .map_err(invalid_request)?
            .to_string();
        if let Some(engine) = &self.evidence_engine {
            crate::tools::task_runtime::require_bound_task(engine, &task_run_id, &project_id)
                .map_err(|_| {
                    AppControlError::new(
                        AppControlErrorCode::TaskBindingMismatch,
                        "File selection requires an existing Task bound to this Project.",
                    )
                })?;
        }
        let grant_id = opaque_id("appfile");
        let mut validation = ScopedFileRoots::new(Vec::new())?;
        validation.add_granted_file(grant_id.clone(), selected_file)?;
        let canonical_file = validation.canonical_granted_file(&grant_id)?;
        let file_name = validation.file_name(&grant_id).ok_or_else(|| {
            AppControlError::new(
                AppControlErrorCode::FileScopeViolation,
                "The selected file name is unavailable.",
            )
        })?;
        let expires_at_ms = self.clock.now_ms().saturating_add(FILE_GRANT_TTL_MS);
        let mut state = self.lock()?;
        state
            .pending_file_grants
            .retain(|_, grant| grant.expires_at_ms >= self.clock.now_ms());
        state.pending_file_grants.insert(
            grant_id.clone(),
            PendingFileGrant {
                project_id,
                task_run_id,
                canonical_file,
                expires_at_ms,
            },
        );
        Ok(AppControlFileGrantView {
            grant_id,
            file_name,
            expires_at_ms,
        })
    }

    pub fn start_session(
        &self,
        request: StartAppControlSession,
    ) -> AppControlResult<AppControlSessionView> {
        let project_id = ProjectId::parse(request.project_id)
            .map_err(invalid_request)?
            .to_string();
        let task_run_id = TaskRunId::parse(request.task_run_id)
            .map_err(invalid_request)?
            .to_string();
        if let Some(engine) = &self.evidence_engine {
            crate::tools::task_runtime::require_bound_task(engine, &task_run_id, &project_id)
                .map_err(|_| {
                    AppControlError::new(
                        AppControlErrorCode::TaskBindingMismatch,
                        "App control requires an existing Task bound to this Project.",
                    )
                })?;
        }
        if request.approved_bundle_ids.is_empty() || request.approved_bundle_ids.len() > 16 {
            return Err(invalid_request(
                "App control requires one to sixteen approved applications.",
            ));
        }
        let mut approved_bundle_ids = HashSet::new();
        for bundle_id in request.approved_bundle_ids {
            if !valid_bundle_id(&bundle_id) {
                return Err(invalid_request(
                    "An approved application identifier is invalid.",
                ));
            }
            if app_profile(&bundle_id, &bundle_id).qualification
                == ApplicationQualification::Browser
            {
                return Err(AppControlError::new(
                    AppControlErrorCode::BrowserRouteRequired,
                    "Browser work must use the guarded browser runtime.",
                ));
            }
            approved_bundle_ids.insert(bundle_id);
        }
        validate_file_grant_ids(&request.file_grant_ids)?;
        let mut file_roots = ScopedFileRoots::new(request.scoped_file_roots)?;
        let now = self.clock.now_ms();
        let mut state = self.lock()?;
        let grants = request
            .file_grant_ids
            .iter()
            .map(|grant_id| {
                let grant = state
                    .pending_file_grants
                    .get(grant_id)
                    .filter(|grant| {
                        grant.project_id == project_id
                            && grant.task_run_id == task_run_id
                            && grant.expires_at_ms >= now
                    })
                    .cloned()
                    .ok_or_else(|| {
                        AppControlError::new(
                            AppControlErrorCode::FileScopeViolation,
                            "A selected file grant is expired or belongs to another Task.",
                        )
                    })?;
                Ok((grant_id.clone(), grant))
            })
            .collect::<AppControlResult<Vec<_>>>()?;
        for (grant_id, grant) in &grants {
            file_roots.add_granted_file(grant_id.clone(), grant.canonical_file.clone())?;
        }
        for (grant_id, _) in &grants {
            state.pending_file_grants.remove(grant_id);
        }
        let session_id = opaque_id("appcontrol");
        let record = SessionRecord {
            session_id: session_id.clone(),
            project_id,
            task_run_id,
            approved_bundle_ids,
            file_roots,
            state: AppControlState::Observing,
            generation: 1,
            revision: 0,
            current_action: None,
            pause_reason: None,
            last_outcome: None,
            last_observation: None,
            updated_at_ms: now,
            cancellation_epoch: Arc::new(AtomicU64::new(1)),
            mismatch_count: 0,
        };
        let view = session_view(&record);
        state.active_session_id = Some(session_id.clone());
        state.sessions.insert(session_id, record);
        drop(state);
        self.record_task_event(
            &view.task_run_id,
            "app_control.session_started",
            crate::p0_contracts::EvidenceClass::ObservedResult,
            serde_json::json!({
                "sessionId": view.session_id,
                "projectId": view.project_id,
                "observationGeneration": view.observation_generation,
            }),
        )?;
        Ok(view)
    }

    pub fn get_status(
        &self,
        task_run_id: Option<&str>,
    ) -> AppControlResult<Option<AppControlSessionView>> {
        let state = self.lock()?;
        if let Some(task_run_id) = task_run_id {
            TaskRunId::parse(task_run_id).map_err(invalid_request)?;
            return Ok(state
                .sessions
                .values()
                .filter(|session| session.task_run_id == task_run_id)
                .max_by_key(|session| session.updated_at_ms)
                .map(session_view));
        }
        let now = self.clock.now_ms();
        let active = state
            .active_session_id
            .as_ref()
            .and_then(|id| state.sessions.get(id))
            .filter(|session| {
                session.state.active()
                    || now.saturating_sub(session.updated_at_ms) <= TERMINAL_STATUS_GRACE_MS
            });
        Ok(active.map(session_view))
    }

    pub fn control(
        &self,
        request: ControlAppControlSessionRequest,
    ) -> AppControlResult<AppControlSessionView> {
        let now = self.clock.now_ms();
        let mut state = self.lock()?;
        let ManagerState {
            sessions,
            references,
            active_session_id,
            ..
        } = &mut *state;
        let session = sessions
            .get_mut(&request.session_id)
            .ok_or_else(session_not_found)?;
        require_task(session, &request.task_run_id)?;
        match request.control {
            AppControlControl::Pause => {
                if !matches!(
                    session.state,
                    AppControlState::Observing
                        | AppControlState::Running
                        | AppControlState::ReturnPending
                ) {
                    return Err(not_running());
                }
                invalidate_generation(session, references, now);
                session.state = AppControlState::Paused;
                session.pause_reason = Some(AppControlPauseReason::UserInput);
            }
            AppControlControl::Stop => {
                if !session.state.active() {
                    return Err(not_running());
                }
                invalidate_generation(session, references, now);
                session.state = AppControlState::Stopped;
                session.pause_reason = None;
                if active_session_id.as_deref() != Some(&session.session_id) {
                    *active_session_id = Some(session.session_id.clone());
                }
            }
            AppControlControl::TakeControl => {
                if !session.state.active() || session.state == AppControlState::Takeover {
                    return Err(not_running());
                }
                invalidate_generation(session, references, now);
                session.state = AppControlState::Takeover;
                session.pause_reason = Some(AppControlPauseReason::UserInput);
            }
            AppControlControl::ReturnToOomu => {
                if !matches!(
                    session.state,
                    AppControlState::Takeover | AppControlState::Paused
                ) {
                    return Err(not_running());
                }
                invalidate_generation(session, references, now);
                session.state = AppControlState::ReturnPending;
                session.pause_reason = None;
            }
        }
        Ok(session_view(session))
    }

    pub fn notify_user_input(
        &self,
        session_id: &str,
        task_run_id: &str,
    ) -> AppControlResult<AppControlSessionView> {
        let now = self.clock.now_ms();
        let mut state = self.lock()?;
        let ManagerState {
            sessions,
            references,
            ..
        } = &mut *state;
        let session = sessions.get_mut(session_id).ok_or_else(session_not_found)?;
        require_task(session, task_run_id)?;
        if !session.state.active() {
            return Err(not_running());
        }
        invalidate_generation(session, references, now);
        session.state = AppControlState::Paused;
        session.pause_reason = Some(AppControlPauseReason::UserInput);
        Ok(session_view(session))
    }
}

fn validate_file_grant_ids(grant_ids: &[String]) -> AppControlResult<()> {
    if grant_ids.len() > 16
        || grant_ids
            .iter()
            .any(|grant_id| !valid_file_grant_id(grant_id))
        || grant_ids.iter().collect::<HashSet<_>>().len() != grant_ids.len()
    {
        Err(AppControlError::new(
            AppControlErrorCode::FileScopeViolation,
            "The selected file grants are invalid.",
        ))
    } else {
        Ok(())
    }
}
