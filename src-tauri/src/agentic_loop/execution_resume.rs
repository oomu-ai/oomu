use super::*;

mod permission_continuation;

impl AgentPlanExecutionTurnContext {
    pub(super) fn persistence_context(
        &self,
    ) -> Result<ChatTurnPersistenceContext, AgenticLoopError> {
        if self.created_at_ms == 0
            || self.attachment_grants.iter().any(|grant| {
                grant.name.trim().is_empty()
                    || grant.mime_type.trim().is_empty()
                    || grant.name.len() > 512
                    || grant.mime_type.len() > 256
            })
        {
            return Err(AgenticLoopError {
                code: "agent_execution_context_invalid",
                boundary: "ChatTurnPersistence",
                message: "Background execution requires a complete immutable turn context."
                    .to_string(),
                mlc_path: None,
            });
        }
        Ok(ChatTurnPersistenceContext {
            turn_id: self.turn_id.clone(),
            generation_token: self.generation_token.clone(),
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            provider_id: self.provider_id.clone(),
            model_id: self.model_id.clone(),
            parent_turn_id: self.parent_turn_id.clone(),
            root_turn_id: self.root_turn_id.clone(),
            turn_kind: self.turn_kind.clone(),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeAgentExecutionRequest {
    pub execution_id: String,
    #[serde(default)]
    pub permission_continuation_capability_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeAfterPermissionRequest {
    pub capability_id: String,
    #[serde(default)]
    pub execution_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeAfterPermissionResponse {
    pub resumed: bool,
    pub execution_id: Option<String>,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareAgentExecutionReplanRequest {
    pub execution_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelAgentExecutionRemainingWorkRequest {
    pub execution_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelAgentExecutionRemainingWorkResponse {
    pub status: &'static str,
    pub completed_step_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareAgentExecutionReplanResponse {
    pub objective: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarRecoveryResolution {
    SelectExisting,
    CreateRequested,
    Cancel,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveAgentCalendarRecoveryRequest {
    pub execution_id: String,
    pub session_id: String,
    pub resolution: CalendarRecoveryResolution,
    #[serde(default)]
    pub calendar_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveAgentCalendarRecoveryResponse {
    pub status: &'static str,
    pub selected_calendar_name: Option<String>,
}

pub(super) fn durable_request_json(
    request: &AgentPlanExecutionRequest,
) -> Result<String, AgenticLoopError> {
    let mut durable_request = request.clone();
    durable_request.authority_proof_id = None;
    serde_json::to_string(&durable_request).map_err(|error| AgenticLoopError {
        code: "agent_execution_context_invalid",
        boundary: "ChatTurnPersistence",
        message: error.to_string(),
        mlc_path: None,
    })
}

pub(super) fn serialize_plan_for_persistence(
    plan: &ActionPlan,
) -> Result<String, AgenticLoopError> {
    serde_json::to_string(plan).map_err(|error| AgenticLoopError {
        code: "plan_serialization_failed",
        boundary: "PersistentStateEngine",
        message: error.to_string(),
        mlc_path: None,
    })
}

#[derive(Clone)]
pub(super) struct AgentExecutionOriginGuard {
    pub(super) execution_id: String,
    pub(super) plan_id: String,
    pub(super) context: ChatTurnPersistenceContext,
    pub(super) context_json: String,
    pub(super) persistence: PersistenceEngine,
    pub(super) stream_start_after_log_id: i64,
}

impl AgentExecutionOriginGuard {
    pub(super) fn begin(
        execution_id: String,
        request: &mut AgentPlanExecutionRequest,
        persistence: PersistenceEngine,
    ) -> Result<Self, AgenticLoopError> {
        ensure_execution_project_scope(request, &persistence)?;
        let requested_context = request.turn_context.persistence_context()?;
        persistence
            .ensure_chat_turn_for_native_action(&requested_context)
            .map_err(agent_execution_origin_error)?;
        let context = persistence
            .canonical_agent_execution_origin_context(&requested_context)
            .map_err(agent_execution_origin_error)?;
        request.turn_context.provider_id = context.provider_id.clone();
        request.turn_context.model_id = context.model_id.clone();
        let context_json = durable_request_json(request)?;
        persistence
            .begin_agent_execution(&execution_id, &request.plan.id, &context, &context_json)
            .map_err(agent_execution_origin_error)?;
        Ok(Self {
            execution_id,
            plan_id: request.plan.id.clone(),
            context,
            context_json,
            persistence,
            stream_start_after_log_id: 0,
        })
    }

    pub(super) fn ensure_current(&self) -> Result<(), AgenticLoopError> {
        self.persistence
            .validate_agent_execution_origin(
                &self.execution_id,
                &self.plan_id,
                &self.context,
                &self.context_json,
            )
            .map_err(agent_execution_origin_error)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finalize(
        &self,
        terminal_status: &str,
        receipt: Option<&str>,
        log_level: &str,
        log_phase: &str,
        log_message: &str,
        payload_json: Option<&str>,
    ) -> Result<(), AgenticLoopError> {
        self.persistence
            .finalize_agent_execution(
                &self.execution_id,
                &self.plan_id,
                &self.context,
                &self.context_json,
                terminal_status,
                receipt,
                log_level,
                log_phase,
                log_message,
                payload_json,
            )
            .map_err(agent_execution_origin_error)
    }
}

pub(super) fn agent_execution_origin_error(error: impl ToString) -> AgenticLoopError {
    let detail = error.to_string();
    let already_started = detail.contains("already exists for this plan origin");
    let stale = detail.contains("stale")
        || detail.contains("cancelled")
        || detail.contains("deleted")
        || detail.contains("does not match")
        || detail.contains("immutable ownership")
        || detail.contains("originating turn")
        || detail.contains("Query returned no rows");
    AgenticLoopError {
        code: if already_started {
            "agent_execution_already_started"
        } else if stale {
            "agent_execution_origin_stale"
        } else {
            "agent_execution_persistence_failed"
        },
        boundary: "ChatTurnPersistence",
        message: if already_started {
            "This plan has already started. OOMU did not start it twice.".to_string()
        } else if stale {
            "This plan no longer matches the message that created it. Nothing was changed. Create a fresh plan and try again."
                .to_string()
        } else {
            eprintln!("OOMU_AGENT_EXECUTION_ORIGIN_PERSISTENCE_FAILED error={detail}");
            "OOMU couldn’t securely start this plan. Nothing was changed. Try again.".to_string()
        },
        mlc_path: None,
    }
}

fn resume_origin_guard(
    execution_id: String,
    request: &AgentPlanExecutionRequest,
    persistence: PersistenceEngine,
) -> Result<AgentExecutionOriginGuard, AgenticLoopError> {
    ensure_execution_project_scope(request, &persistence)?;
    let context = request.turn_context.persistence_context()?;
    let context_json = durable_request_json(request)?;
    let stream_start_after_log_id = persistence
        .resume_agent_execution(&execution_id, &request.plan.id, &context, &context_json)
        .map_err(agent_execution_origin_error)?;
    Ok(AgentExecutionOriginGuard {
        execution_id,
        plan_id: request.plan.id.clone(),
        context,
        context_json,
        persistence,
        stream_start_after_log_id,
    })
}

fn verified_resume_step_index(
    request: &AgentPlanExecutionRequest,
    persistence: &PersistenceEngine,
) -> Result<usize, AgenticLoopError> {
    let plan_json = serialize_plan_for_persistence(&request.plan)?;
    persistence
        .load_plan_execution_checkpoint(&request.plan.id, &plan_json, request.plan.steps.len())
        .map(|checkpoint| checkpoint.map_or(0, |value| value.next_step_index))
        .map_err(|message| AgenticLoopError {
            code: "execution_checkpoint_invalid",
            boundary: "PersistentStateEngine",
            message: format!(
                "OOMU could not safely resume this plan because its execution checkpoint did not match the signed plan ({message}). Nothing was replayed."
            ),
            mlc_path: None,
        })
}

#[tauri::command]
pub async fn request_agent_plan_authority(
    request: RequestAgentPlanAuthority,
    app: tauri::AppHandle,
    identity: tauri::State<'_, SovereignIdentity>,
    authority: tauri::State<'_, crate::authority::NativeAuthorityManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<AgentPlanAuthorityResponse, AgenticLoopError> {
    validate_plan_execution_origin(&request.request, persistence.inner())?;
    issue_agent_plan_authority(
        &request.request,
        request.locale,
        0,
        &app,
        identity.inner(),
        authority.inner(),
    )
    .await
}

fn validate_plan_execution_origin(
    request: &AgentPlanExecutionRequest,
    persistence: &PersistenceEngine,
) -> Result<ChatTurnPersistenceContext, AgenticLoopError> {
    ensure_execution_project_scope(request, persistence)?;
    let requested = request.turn_context.persistence_context()?;
    persistence
        .ensure_chat_turn_for_native_action(&requested)
        .and_then(|_| persistence.canonical_agent_execution_origin_context(&requested))
        .map_err(agent_execution_origin_error)
}

fn ensure_execution_project_scope(
    request: &AgentPlanExecutionRequest,
    persistence: &PersistenceEngine,
) -> Result<(), AgenticLoopError> {
    let session = persistence
        .select_chat_session_by_id(&request.turn_context.session_id)
        .map_err(agent_execution_origin_error)?;
    let terminal_cwds = request
        .plan
        .steps
        .iter()
        .filter_map(|step| match &step.tool {
            Tool::TerminalExecute { cwd, .. } => Some(cwd.as_deref()),
            _ => None,
        });
    validate_project_binding(
        session.project_id.as_deref(),
        request.turn_context.project_id.as_deref(),
        terminal_cwds,
    )
}

fn validate_project_binding<'a>(
    stored_project_id: Option<&str>,
    requested_project_id: Option<&str>,
    terminal_cwds: impl IntoIterator<Item = Option<&'a str>>,
) -> Result<(), AgenticLoopError> {
    if stored_project_id != requested_project_id {
        return Err(project_scope_error(
            "This plan no longer belongs to this conversation’s Project. Create a fresh plan and try again.",
        ));
    }
    let terminal_cwds = terminal_cwds.into_iter().collect::<Vec<_>>();
    if terminal_cwds
        .iter()
        .any(|cwd| cwd.map(str::trim).unwrap_or_default().is_empty())
    {
        return Err(project_scope_error(
            "Choose a Project folder and create a fresh plan before terminal work runs.",
        ));
    }
    Ok(())
}

fn project_scope_error(message: &str) -> AgenticLoopError {
    AgenticLoopError {
        code: "agent_execution_project_scope_stale",
        boundary: "ProjectTerminalScope",
        message: message.to_string(),
        mlc_path: None,
    }
}

async fn issue_agent_plan_authority(
    request: &AgentPlanExecutionRequest,
    locale: Option<String>,
    first_uncompleted_step: usize,
    app: &tauri::AppHandle,
    identity: &SovereignIdentity,
    authority: &crate::authority::NativeAuthorityManager,
) -> Result<AgentPlanAuthorityResponse, AgenticLoopError> {
    let actuation =
        approved_agent_plan_actuation_budget(request, identity, first_uncompleted_step)?;
    if actuation.max_steps == 0 {
        return Ok(AgentPlanAuthorityResponse {
            authority_proof_id: None,
            expires_at_ms: None,
        });
    }
    let session_id = request.turn_context.session_id.trim().to_string();
    if session_id.is_empty() {
        return Err(AgenticLoopError {
            code: "authority_session_required",
            boundary: "NativeAuthorityBoundary",
            message: "This action is not attached to an active session.".to_string(),
            mlc_path: None,
        });
    }
    let actor_id =
        crate::authority::current_actor_id(identity).map_err(|error| AgenticLoopError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
            mlc_path: None,
        })?;
    #[cfg(debug_assertions)]
    {
        let context = &request.turn_context;
        let exact_plan_contract =
            crate::agentic_loop::scenario_plan::matches_scenario_one_deterministic_plan(
                &request.plan,
                crate::scenario_one_e2e_profile::output_directory(),
            );
        crate::scenario_one_e2e_profile::arm_exact_plan_authority(
            &crate::scenario_one_e2e_profile::PlanAuthorityProbe {
                session_id: &context.session_id,
                turn_id: &context.turn_id,
                generation_token: &context.generation_token,
                root_turn_id: &context.root_turn_id,
                created_at_ms: context.created_at_ms,
                automated_web_grounding_enabled: context.automated_web_grounding_enabled,
                model_id: &context.model_id,
                principal_approved: request.principal_approved,
                authority_proof_absent: request.authority_proof_id.is_none(),
                trusted_automatic_execution: request.plan.trusted_automatic_execution,
                exact_plan_contract,
            },
        );
    }
    let response = authority
        .request_after_native_presence(
            app,
            actor_id,
            crate::authority::RequestNativeAuthorityProof {
                session_id: session_id.clone(),
                operation_classes: actuation.operation_classes,
                scopes: vec![format!("actuation-session:{session_id}")],
                max_steps: actuation.max_steps,
                persistence: "session_gated".to_string(),
                locale,
            },
        )
        .await
        .map_err(|error| AgenticLoopError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
            mlc_path: None,
        })?;
    Ok(AgentPlanAuthorityResponse {
        authority_proof_id: Some(response.proof_id),
        expires_at_ms: Some(response.expires_at_ms),
    })
}

#[tauri::command]
pub async fn resume_agent_execution(
    request: ResumeAgentExecutionRequest,
    app: tauri::AppHandle,
    agent_manager: tauri::State<'_, AgentManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
    memory_ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
    gemma: tauri::State<'_, GemmaService>,
    leases: tauri::State<'_, ActuationLeaseManager>,
    authority: tauri::State<'_, crate::authority::NativeAuthorityManager>,
) -> Result<AgentExecutionStartResponse, AgenticLoopError> {
    require_durable_execution(persistence.inner(), "resumed agent execution")?;
    let execution_id = request.execution_id.trim().to_string();
    if execution_id.is_empty() {
        return Err(AgenticLoopError {
            code: "agent_execution_resume_invalid",
            boundary: "ChatTurnPersistence",
            message: "The execution to resume is missing.".to_string(),
            mlc_path: None,
        });
    }
    let persistence_engine = persistence.inner().clone();
    let durable_request_json = persistence_engine
        .load_resumable_agent_execution_request(&execution_id)
        .map_err(agent_execution_origin_error)?;
    let mut execution_request =
        serde_json::from_str::<AgentPlanExecutionRequest>(&durable_request_json).map_err(|_| {
            AgenticLoopError {
                code: "agent_execution_resume_invalid",
                boundary: "ChatTurnPersistence",
                message: "This execution predates safe checkpoint recovery and cannot be resumed."
                    .to_string(),
                mlc_path: None,
            }
        })?;
    execution_request.authority_proof_id = None;
    validate_plan_execution_origin(&execution_request, &persistence_engine)?;
    let agent_id = execution_request.turn_context.agent_id.clone();
    let session_id = execution_request.turn_context.session_id.clone();
    let agent = agent_manager
        .get_active_agent_config(agent_id.clone())
        .await
        .map_err(|message| AgenticLoopError {
            code: "agent_config_load_failed",
            boundary: "AgentManager",
            message,
            mlc_path: None,
        })?
        .ok_or_else(|| AgenticLoopError {
            code: "agent_config_not_found",
            boundary: "AgentManager",
            message: format!("No active agent config found for {agent_id}."),
            mlc_path: None,
        })?;
    let locale = crate::settings::locale_state_for_engine(&persistence_engine, None)
        .map(|state| state.active_locale)
        .ok();
    let resume_step_index = verified_resume_step_index(&execution_request, &persistence_engine)?;
    let actuation_budget = approved_agent_plan_actuation_budget(
        &execution_request,
        identity.inner(),
        resume_step_index,
    )?;
    let permission_continuation = permission_continuation::prepare(
        request.permission_continuation_capability_id.as_deref(),
        &persistence_engine,
        &execution_id,
    )
    .await?;
    let origin_guard = resume_origin_guard(
        execution_id.clone(),
        &execution_request,
        persistence_engine.clone(),
    )?;
    if let Some(continuation) = permission_continuation {
        if let Err(error) = permission_continuation::record(continuation, &persistence_engine) {
            let _ = recovery::finalize_error(
                &origin_guard,
                &persistence_engine,
                &execution_request.plan,
                &error,
                "permission_continuation_failed",
            );
            return Err(error);
        }
    }
    let authority_response = match issue_agent_plan_authority(
        &execution_request,
        locale.clone(),
        resume_step_index,
        &app,
        identity.inner(),
        authority.inner(),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            let _ = recovery::finalize_error(
                &origin_guard,
                &persistence_engine,
                &execution_request.plan,
                &error,
                "resume_authority_failed",
            );
            return Err(error);
        }
    };
    execution_request.authority_proof_id = authority_response.authority_proof_id;
    let auto_turn_registration = background_execution::completion_registration(
        &execution_request,
        &execution_id,
        locale.unwrap_or_else(|| "en-US".to_string()),
    );
    if let Err(error) = background_execution::register_completion(&app, &auto_turn_registration) {
        let _ = recovery::finalize_error(
            &origin_guard,
            &persistence_engine,
            &execution_request.plan,
            &error,
            "auto_turn_registration_failed",
        );
        return Err(error);
    }
    if let Err(error) = provision_approved_agent_plan_lease(
        &execution_request,
        &actuation_budget,
        leases.inner(),
        authority.inner(),
        identity.inner(),
        &app,
    ) {
        background_execution::cancel_completion(&app, &execution_id);
        let _ = recovery::finalize_error(
            &origin_guard,
            &persistence_engine,
            &execution_request.plan,
            &error,
            "resume_lease_failed",
        );
        return Err(error);
    }
    let response = AgentExecutionStartResponse {
        execution_id: execution_id.clone(),
        plan_id: execution_request.plan.id.clone(),
        session_id: session_id.clone(),
        stream_start_after_log_id: origin_guard.stream_start_after_log_id,
    };
    background_execution::spawn(
        execution_request,
        agent,
        persistence_engine,
        memory_ledger.inner().clone(),
        identity.inner().clone(),
        gemma.inner().clone(),
        execution_id,
        leases.inner().clone(),
        app,
        origin_guard,
        auto_turn_registration,
    );
    Ok(response)
}

#[tauri::command]
pub async fn resume_agent_execution_after_permission(
    request: ResumeAfterPermissionRequest,
    app: tauri::AppHandle,
    agent_manager: tauri::State<'_, AgentManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
    memory_ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
    gemma: tauri::State<'_, GemmaService>,
    leases: tauri::State<'_, ActuationLeaseManager>,
    authority: tauri::State<'_, crate::authority::NativeAuthorityManager>,
) -> Result<ResumeAfterPermissionResponse, AgenticLoopError> {
    let capability_id = request.capability_id.trim().to_string();
    let requested_execution_id = request
        .execution_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let waiting = permission_continuation::candidates(
        persistence.inner(),
        &capability_id,
        requested_execution_id,
    )?;
    if waiting.len() != 1 {
        return Ok(ResumeAfterPermissionResponse {
            resumed: false,
            execution_id: None,
            reason: if waiting.is_empty() {
                "no_waiting_work"
            } else {
                "choose_waiting_work"
            },
        });
    }
    let execution_id = waiting[0].clone();
    let permission = crate::macos_permission_broker::status_for_operation(&capability_id).await;
    if !permission_continuation::state_can_continue(permission.state) {
        return Ok(ResumeAfterPermissionResponse {
            resumed: false,
            execution_id: Some(execution_id),
            reason: "permission_not_allowed",
        });
    }
    match resume_agent_execution(
        ResumeAgentExecutionRequest {
            execution_id: execution_id.clone(),
            permission_continuation_capability_id: Some(capability_id.clone()),
        },
        app,
        agent_manager,
        persistence,
        memory_ledger,
        identity,
        gemma,
        leases,
        authority,
    )
    .await
    {
        Ok(_) => Ok(ResumeAfterPermissionResponse {
            resumed: true,
            execution_id: Some(execution_id),
            reason: "resumed",
        }),
        Err(error) if error.code == "agent_execution_origin_stale" => {
            Ok(ResumeAfterPermissionResponse {
                resumed: false,
                execution_id: None,
                reason: "already_resumed",
            })
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub fn cancel_agent_execution_remaining_work(
    request: CancelAgentExecutionRemainingWorkRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<CancelAgentExecutionRemainingWorkResponse, AgenticLoopError> {
    cancel_agent_execution_remaining_work_inner(request, persistence.inner())
}

fn cancel_agent_execution_remaining_work_inner(
    request: CancelAgentExecutionRemainingWorkRequest,
    persistence: &PersistenceEngine,
) -> Result<CancelAgentExecutionRemainingWorkResponse, AgenticLoopError> {
    require_durable_execution(persistence, "agent execution cancellation")?;
    let execution_id = request.execution_id.trim();
    let session_id = request.session_id.trim();
    if execution_id.is_empty() || session_id.is_empty() {
        return Err(AgenticLoopError {
            code: "agent_execution_cancel_invalid",
            boundary: "ChatTurnPersistence",
            message: "This stopped work is not attached to the current conversation.".to_string(),
            mlc_path: None,
        });
    }
    let completed_step_count = persistence
        .cancel_agent_execution_remaining_work(execution_id, session_id)
        .map_err(|error| {
            eprintln!(
                "OOMU_AGENT_EXECUTION_REMAINING_WORK_CANCEL_FAILED execution_id={} error={}",
                crate::redaction::redacted_log_text(execution_id),
                crate::redaction::redacted_log_text(&error.to_string()),
            );
            AgenticLoopError {
                code: "agent_execution_cancel_failed",
                boundary: "ChatTurnPersistence",
                message: "OOMU couldn’t safely stop the remaining work. Nothing was replayed."
                    .to_string(),
                mlc_path: None,
            }
        })?;
    Ok(CancelAgentExecutionRemainingWorkResponse {
        status: "cancelled",
        completed_step_count,
    })
}

#[tauri::command]
pub fn prepare_agent_execution_replan(
    request: PrepareAgentExecutionReplanRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<PrepareAgentExecutionReplanResponse, AgenticLoopError> {
    let execution_id = request.execution_id.trim();
    let session_id = request.session_id.trim();
    if execution_id.is_empty() || session_id.is_empty() {
        return Err(AgenticLoopError {
            code: "agent_execution_replan_invalid",
            boundary: "ChatTurnPersistence",
            message: "This stopped work is not attached to the current conversation.".to_string(),
            mlc_path: None,
        });
    }
    let objective = persistence
        .prepare_agent_execution_replan(execution_id, session_id)
        .map_err(agent_execution_origin_error)?
        .ok_or_else(|| AgenticLoopError {
            code: "agent_execution_replan_unavailable",
            boundary: "ChatTurnPersistence",
            message: "OOMU cannot safely start a new plan from this stopped work. Review the saved details before continuing."
                .to_string(),
            mlc_path: None,
        })?;
    Ok(PrepareAgentExecutionReplanResponse { objective })
}

fn calendar_recovery_error(code: &'static str, message: impl Into<String>) -> AgenticLoopError {
    AgenticLoopError {
        code,
        boundary: "CalendarRecovery",
        message: message.into(),
        mlc_path: None,
    }
}

fn calendar_recovery_target_error(
    failure: crate::tools::eventkit_calendar::CalendarReadFailure,
    fallback_code: &'static str,
) -> AgenticLoopError {
    let code = match failure.code.as_str() {
        "calendar_not_found" => "calendar_not_found",
        "calendar_name_ambiguous" => "calendar_name_ambiguous",
        "calendar_read_only" => "calendar_read_only",
        "calendar_availability_unsupported" => "calendar_availability_unsupported",
        "calendar_permission_denied" => "calendar_permission_denied",
        "calendar_permission_restricted" => "calendar_permission_restricted",
        "calendar_permission_write_only" => "calendar_permission_write_only",
        "calendar_permission_unavailable" => "calendar_permission_unavailable",
        "calendar_authorization_timeout" => "calendar_authorization_timeout",
        "calendar_cleanup_failed" => "calendar_cleanup_failed",
        "calendar_source_unavailable" => "calendar_source_unavailable",
        "calendar_create_failed" => "calendar_create_failed",
        "calendar_create_verification_failed" => "calendar_create_verification_failed",
        _ => fallback_code,
    };
    calendar_recovery_error(code, failure.message)
}

fn calendar_creation_approval(
    request: &AgentPlanExecutionRequest,
    execution_id: &str,
    calendar_name: &str,
) -> crate::shield_gate::ShieldApprovalRequest {
    use rand_core::{OsRng, RngCore};
    let mut token = [0_u8; 18];
    OsRng.fill_bytes(&mut token);
    crate::shield_gate::ShieldApprovalRequest {
        approval_token: format!("approval_{}", hex::encode(token)),
        session_id: Some(request.turn_context.session_id.clone()),
        turn_id: Some(request.turn_context.turn_id.clone()),
        generation_token: Some(request.turn_context.generation_token.clone()),
        action_type: "create_system_calendar".to_string(),
        action_label: "Create a calendar".to_string(),
        target_path: None,
        principal: Some(request.turn_context.agent_id.clone()),
        risk_tier: "consequential".to_string(),
        reason: "Create the exact calendar requested by this paused task.".to_string(),
        estimated_token_costs: None,
        requested_at_ms: crate::foundation::clock::unix_time_ms_u64(),
        preview: serde_json::json!({ "calendarName": calendar_name }).to_string(),
        semantic_summary: format!("Create the calendar “{calendar_name}”"),
        semantic_detail: "This creates one empty calendar. The paused event step remains separate."
            .to_string(),
        approval_tier: "effectful".to_string(),
        approval_mode: "single_exact_calendar".to_string(),
        diff_preview: None,
        scope_trust_available: false,
        scope_trust_prefix: None,
        scope_trust_duration_ms: 0,
        project_id: None,
        task_run_id: Some(execution_id.to_string()),
        action_class: "calendar_create".to_string(),
        argument_class: crate::approval_scopes::argument_class("calendar_create", calendar_name),
        canonical_resource: Some(calendar_name.to_string()),
        mandatory_reconfirm: true,
        approval_scope_kinds: vec!["once".to_string()],
    }
}

fn resolved_calendar_receipt(
    execution_id: &str,
    plan_id: &str,
    requested: &str,
    selected: &str,
    resolution: &str,
    checkpoint_saved: bool,
    previous_plan_sha256: &str,
    resolved_plan_sha256: &str,
) -> String {
    serde_json::json!({
        "schema": recovery::RECOVERY_RECEIPT_SCHEMA,
        "executionId": execution_id,
        "planId": plan_id,
        "code": "calendar_target_resolved",
        "boundary": "CalendarRecovery",
        "recoverable": true,
        "recoveryAction": "resume_same_execution",
        "message": "The calendar target was resolved by the user.",
        "context": {
            "requestedCalendarName": requested,
            "selectedCalendarName": selected,
            "resolution": resolution,
            "amendmentScope": "calendar_name_only",
            "previousPlanSha256": previous_plan_sha256,
            "resolvedPlanSha256": resolved_plan_sha256,
        },
        "changedState": if checkpoint_saved { "checkpoint_saved" } else { "none" },
    })
    .to_string()
}

fn paused_calendar_availability(
    request: &AgentPlanExecutionRequest,
    step_index: usize,
) -> Result<crate::tools::eventkit_calendar::CalendarEventAvailability, AgenticLoopError> {
    let Some(Tool::RegisteredTaskTool(tool)) =
        request.plan.steps.get(step_index).map(|step| &step.tool)
    else {
        return Err(calendar_recovery_error(
            "calendar_recovery_invalid",
            "The paused calendar step no longer matches this choice.",
        ));
    };
    let availability = tool.arguments.get("availability").cloned().ok_or_else(|| {
        calendar_recovery_error(
            "calendar_recovery_invalid",
            "The paused calendar step has no verified availability requirement.",
        )
    })?;
    serde_json::from_value(availability).map_err(|_| {
        calendar_recovery_error(
            "calendar_recovery_invalid",
            "The paused calendar availability requirement is invalid.",
        )
    })
}

#[tauri::command]
pub async fn resolve_agent_calendar_recovery(
    request: ResolveAgentCalendarRecoveryRequest,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
) -> Result<ResolveAgentCalendarRecoveryResponse, AgenticLoopError> {
    require_durable_execution(persistence.inner(), "calendar recovery")?;
    let execution_id = request.execution_id.trim();
    let session_id = request.session_id.trim();
    if execution_id.is_empty() || session_id.is_empty() {
        return Err(calendar_recovery_error(
            "calendar_recovery_invalid",
            "This calendar choice is not attached to the paused task.",
        ));
    }
    if matches!(request.resolution, CalendarRecoveryResolution::Cancel) {
        persistence
            .cancel_agent_calendar_recovery(execution_id, session_id)
            .map_err(agent_execution_origin_error)?;
        return Ok(ResolveAgentCalendarRecoveryResponse {
            status: "cancelled",
            selected_calendar_name: None,
        });
    }
    let (context_json, step_index, requested, available) = persistence
        .load_calendar_recovery_execution_request(execution_id, session_id)
        .map_err(agent_execution_origin_error)?;
    let mut execution_request = serde_json::from_str::<AgentPlanExecutionRequest>(&context_json)
        .map_err(|_| {
            calendar_recovery_error(
                "calendar_recovery_invalid",
                "The paused calendar step could not be read safely.",
            )
        })?;
    let previous_plan_json = serialize_plan_for_persistence(&execution_request.plan)?;
    let required_availability = paused_calendar_availability(&execution_request, step_index)?;
    let (selected, resolution_name, created_calendar_id) = match request.resolution {
        CalendarRecoveryResolution::SelectExisting => {
            let selected = request
                .calendar_name
                .as_deref()
                .map(str::trim)
                .unwrap_or("");
            if !available.iter().any(|name| name == selected) {
                return Err(calendar_recovery_error(
                    "calendar_recovery_selection_invalid",
                    "Choose one of the available writable calendars.",
                ));
            }
            crate::tools::eventkit_calendar::validate_calendar_target(
                selected.to_string(),
                required_availability,
            )
            .await
            .map_err(|failure| {
                calendar_recovery_target_error(failure, "calendar_recovery_selection_invalid")
            })?;
            (selected.to_string(), "selected_existing", None)
        }
        CalendarRecoveryResolution::CreateRequested => {
            crate::shield_gate::request_user_approval(
                &app,
                approvals.inner(),
                calendar_creation_approval(&execution_request, execution_id, &requested),
            )
            .await
            .map_err(|error| {
                calendar_recovery_error(
                    if error.code == "shield_approval_denied" {
                        "calendar_creation_denied"
                    } else {
                        "calendar_creation_approval_failed"
                    },
                    if error.code == "shield_approval_denied" {
                        "The calendar was not created. Choose another calendar or cancel."
                    } else {
                        "OOMU could not ask for permission to create the calendar."
                    },
                )
            })?;
            let calendar_id = crate::tools::eventkit_calendar::create_calendar(
                requested.clone(),
                required_availability,
            )
            .await
            .map_err(|failure| {
                calendar_recovery_target_error(failure, "calendar_creation_failed")
            })?;
            let resolution_name = if calendar_id.is_some() {
                "created_requested"
            } else {
                "requested_already_available"
            };
            (requested.clone(), resolution_name, calendar_id)
        }
        CalendarRecoveryResolution::Cancel => unreachable!("cancel handled before loading"),
    };
    if selected != requested {
        let Some(Tool::RegisteredTaskTool(tool)) = execution_request
            .plan
            .steps
            .get_mut(step_index)
            .map(|step| &mut step.tool)
        else {
            return Err(calendar_recovery_error(
                "calendar_recovery_invalid",
                "The paused calendar step no longer matches this choice.",
            ));
        };
        let Some(arguments) = tool.arguments.as_object_mut() else {
            return Err(calendar_recovery_error(
                "calendar_recovery_invalid",
                "The paused calendar step has invalid arguments.",
            ));
        };
        arguments.insert(
            "calendarName".to_string(),
            serde_json::Value::String(selected.clone()),
        );
        execution_request.plan = sign_plan(execution_request.plan, identity.inner())?;
        MlcVerifier::new()
            .verify_approved_plan(&execution_request.plan, identity.inner())
            .map_err(|error| {
                calendar_recovery_error(
                    "calendar_recovery_plan_invalid",
                    format!(
                        "The resolved calendar step could not be verified: {}",
                        error.message
                    ),
                )
            })?;
    }
    let resolved_context_json = durable_request_json(&execution_request)?;
    let resolved_plan_json = serialize_plan_for_persistence(&execution_request.plan)?;
    let receipt = resolved_calendar_receipt(
        execution_id,
        &execution_request.plan.id,
        &requested,
        &selected,
        resolution_name,
        step_index > 0,
        &crate::foundation::digest::sha256_hex(previous_plan_json.as_bytes()),
        &crate::foundation::digest::sha256_hex(resolved_plan_json.as_bytes()),
    );
    if let Err(error) = persistence.commit_agent_calendar_recovery_resolution(
        execution_id,
        session_id,
        &context_json,
        &resolved_context_json,
        &resolved_plan_json,
        &receipt,
    ) {
        if let Some(calendar_id) = created_calendar_id {
            let _ = crate::tools::eventkit_calendar::remove_calendar(calendar_id).await;
        }
        return Err(agent_execution_origin_error(error));
    }
    Ok(ResolveAgentCalendarRecoveryResponse {
        status: "ready_to_resume",
        selected_calendar_name: Some(selected),
    })
}

pub(super) fn recoverable_agent_execution_error(code: &str) -> bool {
    code == "local_workflow_decision_halted"
        || code == "permission_denied"
        || code == "permission_request_failed"
        || code.starts_with("permission_")
        || code == "action_output_unverified"
        || code.starts_with("authority_")
        || code.starts_with("actuation_lease_")
        || code.starts_with("native_authority_")
        || code == "auto_turn_registration_failed"
        || code == "mlc_verification_failed"
        || code.starts_with("calendar_")
        || code.starts_with("mail_")
        || code.starts_with("decision_pack_")
        || code.starts_with("evidence_artifact_")
        || code.starts_with("registered_task_tool_")
        || code.starts_with("transient_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_recovery_keeps_actionable_native_failure_codes() {
        for (native_code, expected_code) in [
            ("calendar_not_found", "calendar_not_found"),
            ("calendar_read_only", "calendar_read_only"),
            (
                "calendar_availability_unsupported",
                "calendar_availability_unsupported",
            ),
            ("calendar_cleanup_failed", "calendar_cleanup_failed"),
            ("unexpected_eventkit_failure", "calendar_creation_failed"),
        ] {
            let error = calendar_recovery_target_error(
                crate::tools::eventkit_calendar::CalendarReadFailure {
                    code: native_code.to_string(),
                    message: "Safe calendar guidance.".to_string(),
                    retryable: false,
                    requested_calendar_name: Some("OOMU Test".to_string()),
                    available_calendar_names: vec!["Personal".to_string()],
                    receipt: None,
                },
                "calendar_creation_failed",
            );
            assert_eq!(error.code, expected_code, "{native_code}");
            assert_eq!(error.boundary, "CalendarRecovery");
            assert_eq!(error.message, "Safe calendar guidance.");
        }
    }

    #[test]
    fn terminal_authority_requires_the_immutable_project_and_bound_cwd() {
        assert!(validate_project_binding(
            Some("project-a"),
            Some("project-a"),
            [Some("/tmp/project-a")],
        )
        .is_ok());
        assert!(validate_project_binding(None, None, [Some("/approved/external")]).is_ok());
        assert_eq!(
            validate_project_binding(Some("project-a"), Some("project-b"), [])
                .unwrap_err()
                .code,
            "agent_execution_project_scope_stale"
        );
        assert_eq!(
            validate_project_binding(Some("project-a"), Some("project-a"), [None])
                .unwrap_err()
                .code,
            "agent_execution_project_scope_stale"
        );
    }

    #[test]
    fn resume_preparation_failures_remain_recoverable() {
        for code in [
            "authority_user_denied",
            "authority_proof_store_unavailable",
            "authority_identity_unavailable",
            "auto_turn_registration_failed",
            "permission_check_failed",
            "permission_prompt_unavailable",
            "evidence_artifact_preparation_failed",
        ] {
            assert!(
                recoverable_agent_execution_error(code),
                "{code} must leave the same execution resumable"
            );
        }
    }

    #[test]
    fn remaining_work_cancel_command_requires_exact_session_ownership_fields() {
        let temp_dir = std::env::temp_dir().join(format!(
            "oomu-agent-cancel-command-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let persistence = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
        let error = cancel_agent_execution_remaining_work_inner(
            CancelAgentExecutionRemainingWorkRequest {
                execution_id: " ".to_string(),
                session_id: "session-1".to_string(),
            },
            &persistence,
        )
        .unwrap_err();
        assert_eq!(error.code, "agent_execution_cancel_invalid");
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn remaining_work_cancel_command_response_is_frontend_stable() {
        let value = serde_json::to_value(CancelAgentExecutionRemainingWorkResponse {
            status: "cancelled",
            completed_step_count: 2,
        })
        .unwrap();
        assert_eq!(value["status"], "cancelled");
        assert_eq!(value["completedStepCount"], 2);
        assert!(value.get("completed_step_count").is_none());
    }

    fn permission_receipt(execution_id: &str, capability_id: &str, code: &str) -> String {
        serde_json::json!({
            "schema": recovery::RECOVERY_RECEIPT_SCHEMA,
            "executionId": execution_id,
            "planId": "plan-permission",
            "code": code,
            "boundary": "MacosPermissionBroker",
            "recoverable": true,
            "recoveryAction": "resume_same_execution",
            "message": "Permission is needed.",
            "context": { "capabilityId": capability_id },
            "changedState": "none"
        })
        .to_string()
    }

    #[test]
    fn permission_receipts_are_bound_to_one_exact_capability() {
        let mail = permission_receipt(
            "execution-permission",
            "mail",
            "mail_automation_permission_required",
        );
        assert!(permission_continuation::receipt_matches(&mail, "mail"));
        assert!(!permission_continuation::receipt_matches(&mail, "calendar"));
        let mut unsafe_receipt: serde_json::Value = serde_json::from_str(&mail).unwrap();
        unsafe_receipt["recoveryAction"] = serde_json::json!("start_new_plan");
        assert!(!permission_continuation::receipt_matches(
            &unsafe_receipt.to_string(),
            "mail"
        ));
    }

    #[test]
    fn permission_continuation_candidate_is_not_offered_after_it_is_claimed() {
        let temp_dir = std::env::temp_dir().join(format!(
            "oomu-permission-resume-once-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let persistence = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
        let connection = persistence.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO agent_executions (
                execution_id,plan_id,session_id,agent_id,provider_id,model_id,turn_id,
                generation_token,parent_turn_id,root_turn_id,turn_kind,context_json,status,
                created_at_ms,updated_at_ms
             ) VALUES (?1,'plan-permission','session','agent','local','model','turn',
                       'generation',NULL,'turn','root','{}','halted',1,1)",
                rusqlite::params!["execution-permission"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_execution_logs (
                execution_id,plan_id,session_id,agent_id,level,phase,message,payload_json,
                created_at_ms
             ) VALUES (?1,'plan-permission','session','agent','error','terminal',
                       'Permission is needed.',?2,1)",
                rusqlite::params![
                    "execution-permission",
                    permission_receipt(
                        "execution-permission",
                        "mail",
                        "mail_automation_permission_required"
                    )
                ],
            )
            .unwrap();
        assert_eq!(
            permission_continuation::candidates(&persistence, "mail", Some("execution-permission"))
                .unwrap(),
            vec!["execution-permission".to_string()]
        );
        connection.execute(
            "UPDATE agent_executions SET status='running' WHERE execution_id='execution-permission'",
            [],
        ).unwrap();
        assert!(permission_continuation::candidates(
            &persistence,
            "mail",
            Some("execution-permission")
        )
        .unwrap()
        .is_empty());
        drop(connection);
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
