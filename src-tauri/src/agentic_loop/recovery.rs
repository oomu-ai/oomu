use super::*;

pub(crate) const RECOVERY_RECEIPT_SCHEMA: &str = "oomu.agent_execution_recovery.v1";
pub(crate) const ACTION_PREPARED_EFFECTFUL: &str = "prepared_effectful";
pub(crate) const ACTION_PREPARED_READ_ONLY: &str = "prepared_read_only";
pub(crate) const ACTION_STARTED_EFFECTFUL: &str = "started_effectful";
pub(crate) const ACTION_STARTED_READ_ONLY: &str = "started_read_only";
pub(crate) const ACTION_UNVERIFIED_EFFECTFUL: &str = "unverified_effectful";
pub(crate) const ACTION_UNVERIFIED_READ_ONLY: &str = "unverified_read_only";
pub(crate) const ACTION_SENSOR_EFFECTFUL: &str = "sensor_captured_effectful";
pub(crate) const ACTION_SENSOR_READ_ONLY: &str = "sensor_captured_read_only";
pub(crate) const ACTION_FAILED_UNCHANGED_EFFECTFUL: &str = "failed_unchanged_effectful";
pub(crate) const ACTION_FAILED_UNCHANGED_READ_ONLY: &str = "failed_unchanged_read_only";
pub(crate) const ACTION_FAILURE_RECEIPT_SCHEMA: &str = "oomu.agent_action_failure.v1";

pub(super) fn prepared_action_status(potentially_effectful: bool) -> &'static str {
    if potentially_effectful {
        ACTION_PREPARED_EFFECTFUL
    } else {
        ACTION_PREPARED_READ_ONLY
    }
}

pub(super) fn started_action_status(potentially_effectful: bool) -> &'static str {
    if potentially_effectful {
        ACTION_STARTED_EFFECTFUL
    } else {
        ACTION_STARTED_READ_ONLY
    }
}

pub(super) fn unverified_action_status(potentially_effectful: bool) -> &'static str {
    if potentially_effectful {
        ACTION_UNVERIFIED_EFFECTFUL
    } else {
        ACTION_UNVERIFIED_READ_ONLY
    }
}

pub(super) fn sensor_action_status(potentially_effectful: bool) -> &'static str {
    if potentially_effectful {
        ACTION_SENSOR_EFFECTFUL
    } else {
        ACTION_SENSOR_READ_ONLY
    }
}

fn failed_unchanged_action_status(potentially_effectful: bool) -> &'static str {
    if potentially_effectful {
        ACTION_FAILED_UNCHANGED_EFFECTFUL
    } else {
        ACTION_FAILED_UNCHANGED_READ_ONLY
    }
}

pub(crate) fn verified_unchanged_action_receipt(status: &str, output: Option<&str>) -> bool {
    let potentially_effectful = match status {
        ACTION_FAILED_UNCHANGED_EFFECTFUL => true,
        ACTION_FAILED_UNCHANGED_READ_ONLY => false,
        _ => return false,
    };
    let Some(receipt) =
        output.and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
    else {
        return false;
    };
    let Some(operation) = receipt.get("operation").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let Some(agent_error) = receipt
        .get("agentError")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    receipt.get("schema").and_then(serde_json::Value::as_str) == Some(ACTION_FAILURE_RECEIPT_SCHEMA)
        && receipt.get("status").and_then(serde_json::Value::as_str) == Some("failed")
        && receipt.get("verified").and_then(serde_json::Value::as_bool) == Some(true)
        && receipt
            .get("changedState")
            .and_then(serde_json::Value::as_str)
            == Some("none")
        && receipt
            .get("potentiallyEffectful")
            .and_then(serde_json::Value::as_bool)
            == Some(potentially_effectful)
        && crate::tools::task_tool_runtime::parse_retry_safe_unchanged_error(operation, agent_error)
            .is_some()
}

pub(crate) fn automatic_replay_safe_action_status(status: &str) -> bool {
    matches!(
        status,
        ACTION_PREPARED_EFFECTFUL
            | ACTION_PREPARED_READ_ONLY
            | ACTION_STARTED_READ_ONLY
            | ACTION_UNVERIFIED_READ_ONLY
            | ACTION_SENSOR_READ_ONLY
    )
}

pub(super) async fn prepare_agent_action(
    persistence: &PersistenceEngine,
    plan_id: &str,
    operation: &str,
    action: &AuthorizedActions,
    potentially_effectful: bool,
) -> Result<i64, AgenticLoopError> {
    persistence
        .save_action_result(
            plan_id.to_string(),
            operation.to_string(),
            format!("{action:?}"),
            None,
            prepared_action_status(potentially_effectful).to_string(),
        )
        .await
        .map_err(AgenticLoopError::from_persistence)
}

pub(super) async fn record_unverified_agent_action(
    persistence: &PersistenceEngine,
    action_id: i64,
    output: String,
    potentially_effectful: bool,
) -> Result<(), AgenticLoopError> {
    persistence
        .record_agent_action_invocation_result(
            action_id,
            started_action_status(potentially_effectful).to_string(),
            output,
            unverified_action_status(potentially_effectful).to_string(),
        )
        .await
        .map_err(AgenticLoopError::from_persistence)
}

pub(super) async fn record_failed_agent_action(
    persistence: &PersistenceEngine,
    action_id: i64,
    operation: &str,
    error: &AgenticLoopError,
    potentially_effectful: bool,
) -> Result<(), AgenticLoopError> {
    let retry_safe = crate::tools::task_tool_runtime::parse_retry_safe_unchanged_error(
        operation,
        &error.message,
    )
    .is_some();
    if !retry_safe {
        return record_unverified_agent_action(
            persistence,
            action_id,
            error.message.clone(),
            potentially_effectful,
        )
        .await;
    }
    let receipt = serde_json::json!({
        "schema": ACTION_FAILURE_RECEIPT_SCHEMA,
        "operation": operation,
        "status": "failed",
        "verified": true,
        "changedState": "none",
        "potentiallyEffectful": potentially_effectful,
        "agentError": error.message,
    })
    .to_string();
    persistence
        .record_agent_action_invocation_result(
            action_id,
            started_action_status(potentially_effectful).to_string(),
            receipt,
            failed_unchanged_action_status(potentially_effectful).to_string(),
        )
        .await
        .map_err(AgenticLoopError::from_persistence)
}

fn registered_task_result_failure(
    operation: &str,
    result: &crate::shield_gate::ExecuteCommandResponse,
) -> Option<String> {
    if matches!(result.status, crate::shield_gate::CommandStatus::Completed) {
        return None;
    }
    Some(
        if matches!(
            operation,
            "draft_system_email" | "draft_decision_pack_email" | "draft_release_recovery_email"
        ) {
            crate::tools::system_mail::unverified_mail_result_error()
        } else {
            result.message.clone()
        },
    )
}

pub(super) async fn execute_registered_task_tool(
    context: crate::tools::task_tool_runtime::TaskToolExecutionContext<'_>,
    request: crate::tools::task_tool_runtime::ValidatedTaskToolRequest,
    action_id: Option<i64>,
) -> Result<crate::shield_gate::ExecuteCommandResponse, AgenticLoopError> {
    let operation = request.operation;
    let potentially_effectful = request.potentially_effectful();
    let persistence = context.persistence;
    let message = match crate::tools::task_tool_runtime::execute(context, request).await {
        Ok(result) => match registered_task_result_failure(operation, &result) {
            Some(message) => message,
            None => return Ok(result),
        },
        Err(message) => message,
    };
    if crate::scenario_one_e2e_profile::enabled() {
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_TRACE stage=registered_task_tool status=failed operation={operation} error={message}"
        );
    }
    let (code, boundary) = crate::tools::task_tool_runtime::agent_error_metadata(operation);
    let error = AgenticLoopError {
        code,
        boundary,
        message: crate::tools::task_tool_runtime::normalize_agent_error(operation, &message),
        mlc_path: None,
    };
    if let Some(action_id) = action_id {
        record_failed_agent_action(
            persistence,
            action_id,
            operation,
            &error,
            potentially_effectful,
        )
        .await?;
    }
    Err(error)
}

pub(super) async fn record_sensor(
    persistence: &PersistenceEngine,
    action_id: i64,
    output: String,
    potentially_effectful: bool,
) -> Result<(), AgenticLoopError> {
    persistence
        .record_agent_action_invocation_result(
            action_id,
            started_action_status(potentially_effectful).to_string(),
            output,
            sensor_action_status(potentially_effectful).to_string(),
        )
        .await
        .map_err(AgenticLoopError::from_persistence)
}

pub(super) async fn begin_action_invocation(
    persistence: &PersistenceEngine,
    action_id: i64,
    potentially_effectful: bool,
) -> Result<(), AgenticLoopError> {
    persistence
        .mark_agent_action_invocation_started(
            action_id,
            prepared_action_status(potentially_effectful).to_string(),
            started_action_status(potentially_effectful).to_string(),
        )
        .await
        .map_err(AgenticLoopError::from_persistence)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecoveryReceipt {
    schema: &'static str,
    execution_id: String,
    plan_id: String,
    code: String,
    boundary: String,
    recoverable: bool,
    recovery_action: RecoveryAction,
    message: String,
    context: serde_json::Map<String, serde_json::Value>,
    changed_state: crate::tools::task_tool_runtime::TaskToolChangedState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecoveryAction {
    ResumeSameExecution,
    ResolveCalendarTarget,
    StartNewPlan,
    ReviewExternalChanges,
}

impl RecoveryAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::ResumeSameExecution => "resume_same_execution",
            Self::ResolveCalendarTarget => "resolve_calendar_target",
            Self::StartNewPlan => "start_new_plan",
            Self::ReviewExternalChanges => "review_external_changes",
        }
    }
}

impl RecoveryReceipt {
    pub(super) fn from_error(
        execution_id: &str,
        plan: &ActionPlan,
        error: &AgenticLoopError,
        completed_state: crate::tools::task_tool_runtime::TaskToolChangedState,
    ) -> Self {
        let task_error = crate::tools::task_tool_runtime::parse_agent_error(&error.message);
        let (code, boundary, message, context, mut changed_state) = match task_error {
            Some(error) => (
                error.code,
                error.boundary,
                error.message,
                error.context,
                error.changed_state,
            ),
            None => (
                error.code.to_string(),
                error.boundary.to_string(),
                "Execution stopped at a safe boundary.".to_string(),
                serde_json::Map::new(),
                crate::tools::task_tool_runtime::TaskToolChangedState::None,
            ),
        };
        let failed_step_changed_state = changed_state;
        if changed_state == crate::tools::task_tool_runtime::TaskToolChangedState::None {
            changed_state = completed_state;
        }
        let code_is_recoverable = execution_resume::recoverable_agent_execution_error(&code)
            || execution_resume::recoverable_agent_execution_error(error.code);
        let calendar_target_resolution = matches!(
            code.as_str(),
            "calendar_action_denied"
                | "calendar_not_found"
                | "calendar_name_ambiguous"
                | "calendar_read_only"
                | "calendar_availability_unsupported"
        ) && failed_step_changed_state
            == crate::tools::task_tool_runtime::TaskToolChangedState::None
            && changed_state
                != crate::tools::task_tool_runtime::TaskToolChangedState::ExternalChanges;
        let recovery_action = if calendar_target_resolution {
            RecoveryAction::ResolveCalendarTarget
        } else {
            match changed_state {
                crate::tools::task_tool_runtime::TaskToolChangedState::ExternalChanges => {
                    RecoveryAction::ReviewExternalChanges
                }
                crate::tools::task_tool_runtime::TaskToolChangedState::None
                    if !code_is_recoverable =>
                {
                    RecoveryAction::StartNewPlan
                }
                crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved
                    if !code_is_recoverable =>
                {
                    RecoveryAction::ReviewExternalChanges
                }
                _ => RecoveryAction::ResumeSameExecution,
            }
        };
        let recoverable = matches!(
            recovery_action,
            RecoveryAction::ResumeSameExecution | RecoveryAction::ResolveCalendarTarget
        );
        Self {
            schema: RECOVERY_RECEIPT_SCHEMA,
            execution_id: execution_id.to_string(),
            plan_id: plan.id.clone(),
            code,
            boundary,
            recoverable,
            recovery_action,
            message,
            context,
            changed_state,
        }
    }

    pub(super) fn terminal_status(&self) -> &'static str {
        if self.recoverable {
            "halted"
        } else {
            "failed"
        }
    }

    pub(super) fn safe_message(&self) -> &str {
        &self.message
    }

    pub(super) fn recovery_action(&self) -> RecoveryAction {
        self.recovery_action
    }

    pub(super) fn to_json(&self) -> Result<String, AgenticLoopError> {
        serde_json::to_string(self)
            .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
    }
}

fn durable_recovery_state(
    persistence: &PersistenceEngine,
    plan: &ActionPlan,
) -> crate::tools::task_tool_runtime::TaskToolChangedState {
    let Ok(plan_json) = serialize_plan_for_persistence(plan) else {
        return crate::tools::task_tool_runtime::TaskToolChangedState::ExternalChanges;
    };
    let completed =
        match persistence.load_plan_execution_checkpoint(&plan.id, &plan_json, plan.steps.len()) {
            Ok(checkpoint) => checkpoint.map_or(0, |checkpoint| checkpoint.next_step_index),
            Err(_) => {
                return crate::tools::task_tool_runtime::TaskToolChangedState::ExternalChanges
            }
        };
    if persistence
        .has_uncertain_agent_action_effect(&plan.id)
        .unwrap_or(true)
    {
        return crate::tools::task_tool_runtime::TaskToolChangedState::ExternalChanges;
    }
    if completed == 0 {
        return crate::tools::task_tool_runtime::TaskToolChangedState::None;
    }
    crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved
}

pub(super) fn finalize_error(
    origin_guard: &AgentExecutionOriginGuard,
    persistence: &PersistenceEngine,
    plan: &ActionPlan,
    error: &AgenticLoopError,
    phase: &str,
) -> Result<(), AgenticLoopError> {
    let receipt = RecoveryReceipt::from_error(
        &origin_guard.execution_id,
        plan,
        error,
        durable_recovery_state(persistence, plan),
    );
    let terminal_status = receipt.terminal_status();
    let log_phase = if phase == "terminal" {
        terminal_status
    } else {
        phase
    };
    let payload = receipt.to_json()?;
    origin_guard.finalize(
        terminal_status,
        Some(&payload),
        "error",
        log_phase,
        receipt.safe_message(),
        Some(&payload),
    )?;
    eprintln!(
        "OOMU_AGENT_EXECUTION_RECOVERY_FINALIZED code={} boundary={} recovery_action={}",
        crate::redaction::redacted_log_text(&receipt.code),
        crate::redaction::redacted_log_text(&receipt.boundary),
        receipt.recovery_action().as_str(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> ActionPlan {
        ActionPlan {
            id: "plan-recovery".to_string(),
            objective: "Prepare the decision pack".to_string(),
            intent: StructuredIntent {
                objective: "Prepare the decision pack".to_string(),
                category: IntentCategory::ProjectAnalysis,
                source: crate::gemma::IntentSource::Deterministic,
                degraded_reason: None,
            },
            steps: Vec::new(),
            exit_condition: "Complete".to_string(),
            logical_certificate: LogicalCertificate::unsigned(
                Vec::new(),
                Vec::new(),
                String::new(),
            ),
            trusted_automatic_execution: false,
            model_route: ModelRouteDecision {
                selected_model: ModelMetadata::local_gemma(),
                provider_config_id: None,
                provider_id: Some("local_model".to_string()),
                recommended_model: None,
                requires_principal_authorization: false,
                reason: "fixture".to_string(),
                context_excerpt_count: 0,
                context_sources: Vec::new(),
            },
            parent_artifact_hashes: Vec::new(),
        }
    }

    #[test]
    fn typed_task_failure_becomes_frontend_stable_receipt() {
        let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
            "missing",
            r#"{"taskToolError":{"code":"decision_pack_research_evidence_unavailable","message":"Recent official freight evidence was not verified.","context":{"subject":"freight","attemptCount":3,"pageCount":4,"verifiedInputCount":2,"changedState":false}}}"#,
        );
        let error = AgenticLoopError {
            code: "registered_task_tool_failed",
            boundary: "RegisteredTaskTool",
            message: normalized,
            mlc_path: None,
        };
        let receipt = RecoveryReceipt::from_error(
            "execution-1",
            &plan(),
            &error,
            crate::tools::task_tool_runtime::TaskToolChangedState::None,
        );
        let json: serde_json::Value = serde_json::from_str(&receipt.to_json().unwrap()).unwrap();
        assert_eq!(json["schema"], RECOVERY_RECEIPT_SCHEMA);
        assert_eq!(json["executionId"], "execution-1");
        assert_eq!(json["code"], "decision_pack_research_evidence_unavailable");
        assert_eq!(json["context"]["subject"], "freight");
        assert_eq!(json["changedState"], "none");
        assert_eq!(json["recoverable"], true);
        assert_eq!(json["recoveryAction"], "resume_same_execution");
    }

    #[test]
    fn calendar_target_resolution_preserves_completed_checkpoint_without_retrying() {
        let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
            "missing",
            r#"{"taskToolError":{"code":"calendar_not_found","message":"The exact requested calendar was not found.","context":{"requestedCalendarName":"OOMU Test","availableCalendarNames":["Personal"],"changedState":false}}}"#,
        );
        let error = AgenticLoopError {
            code: "calendar_event_failed",
            boundary: "Calendar",
            message: normalized,
            mlc_path: None,
        };
        let receipt = RecoveryReceipt::from_error(
            "execution-calendar",
            &plan(),
            &error,
            crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved,
        );
        let json: serde_json::Value = serde_json::from_str(&receipt.to_json().unwrap()).unwrap();
        assert_eq!(json["changedState"], "checkpoint_saved");
        assert_eq!(json["recoverable"], true);
        assert_eq!(json["recoveryAction"], "resolve_calendar_target");
        assert_eq!(json["context"]["requestedCalendarName"], "OOMU Test");
    }

    #[test]
    fn denied_calendar_action_preserves_checkpoint_and_requests_a_narrow_target_change() {
        let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
            "missing",
            r#"{"taskToolError":{"code":"calendar_action_denied","message":"The Calendar event was not created because you denied this action.","context":{"requestedCalendarName":"Initial Test","availableCalendarNames":["OOMU Test"],"calendarStepArgumentsSha256":"abc123","changedState":false}}}"#,
        );
        let error = AgenticLoopError {
            code: "calendar_action_denied",
            boundary: "ShieldApprovalManager",
            message: normalized,
            mlc_path: None,
        };
        let receipt = RecoveryReceipt::from_error(
            "execution-calendar-denied",
            &plan(),
            &error,
            crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved,
        );
        let json: serde_json::Value = serde_json::from_str(&receipt.to_json().unwrap()).unwrap();
        assert_eq!(json["code"], "calendar_action_denied");
        assert_eq!(json["changedState"], "checkpoint_saved");
        assert_eq!(json["recoverable"], true);
        assert_eq!(json["recoveryAction"], "resolve_calendar_target");
        assert_eq!(json["context"]["requestedCalendarName"], "Initial Test");
        assert_eq!(json["context"]["availableCalendarNames"][0], "OOMU Test");
        assert_eq!(json["context"]["calendarStepArgumentsSha256"], "abc123");
    }

    #[test]
    fn incompatible_calendar_returns_to_capability_filtered_target_resolution() {
        let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
            "unsupported",
            r#"{"taskToolError":{"code":"calendar_availability_unsupported","message":"The requested calendar cannot represent tentative events.","context":{"requestedCalendarName":"OOMU Test","availableCalendarNames":["Calendar"],"changedState":false}}}"#,
        );
        let error = AgenticLoopError {
            code: "calendar_event_failed",
            boundary: "Calendar",
            message: normalized,
            mlc_path: None,
        };
        let receipt = RecoveryReceipt::from_error(
            "execution-calendar-capability",
            &plan(),
            &error,
            crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved,
        );
        let json: serde_json::Value = serde_json::from_str(&receipt.to_json().unwrap()).unwrap();
        assert_eq!(json["changedState"], "checkpoint_saved");
        assert_eq!(json["recoverable"], true);
        assert_eq!(json["recoveryAction"], "resolve_calendar_target");
        assert_eq!(json["context"]["availableCalendarNames"][0], "Calendar");
    }

    #[test]
    fn nonrecoverable_zero_change_failure_can_start_a_new_plan() {
        let error = AgenticLoopError {
            code: "preflight_verification_failed",
            boundary: "MlcVerifier",
            message: "The signed plan could not be verified.".to_string(),
            mlc_path: None,
        };
        let receipt = RecoveryReceipt::from_error(
            "execution-2",
            &plan(),
            &error,
            crate::tools::task_tool_runtime::TaskToolChangedState::None,
        );
        let json: serde_json::Value = serde_json::from_str(&receipt.to_json().unwrap()).unwrap();
        assert_eq!(json["recoverable"], false);
        assert_eq!(json["recoveryAction"], "start_new_plan");
    }

    #[test]
    fn final_verification_retries_only_from_the_completed_checkpoint() {
        let error = AgenticLoopError {
            code: "mlc_verification_failed",
            boundary: "MlcVerifier",
            message: "The final receipt could not be verified.".to_string(),
            mlc_path: None,
        };
        let checkpointed = RecoveryReceipt::from_error(
            "execution-final-verification",
            &plan(),
            &error,
            crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved,
        );
        let json: serde_json::Value =
            serde_json::from_str(&checkpointed.to_json().unwrap()).unwrap();
        assert_eq!(json["changedState"], "checkpoint_saved");
        assert_eq!(json["recoverable"], true);
        assert_eq!(json["recoveryAction"], "resume_same_execution");

        let uncertain = RecoveryReceipt::from_error(
            "execution-final-verification-uncertain",
            &plan(),
            &error,
            crate::tools::task_tool_runtime::TaskToolChangedState::ExternalChanges,
        );
        let json: serde_json::Value = serde_json::from_str(&uncertain.to_json().unwrap()).unwrap();
        assert_eq!(json["recoverable"], false);
        assert_eq!(json["recoveryAction"], "review_external_changes");
    }

    #[test]
    fn external_changes_never_authorize_automatic_replay() {
        let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
            "missing",
            r#"{"taskToolError":{"code":"decision_pack_research_evidence_unavailable","message":"Review the external state before continuing.","context":{"subject":"freight","changedState":true}}}"#,
        );
        let error = AgenticLoopError {
            code: "registered_task_tool_failed",
            boundary: "RegisteredTaskTool",
            message: normalized,
            mlc_path: None,
        };
        let receipt = RecoveryReceipt::from_error(
            "execution-3",
            &plan(),
            &error,
            crate::tools::task_tool_runtime::TaskToolChangedState::None,
        );
        let json: serde_json::Value = serde_json::from_str(&receipt.to_json().unwrap()).unwrap();
        assert_eq!(json["changedState"], "external_changes");
        assert_eq!(json["recoverable"], false);
        assert_eq!(json["recoveryAction"], "review_external_changes");
    }

    #[test]
    fn failed_mail_responses_are_typed_before_the_signer_boundary() {
        let _ = crate::tools::system_mail::register_task_tool();
        let _ = crate::tools::decision_pack_mail::register_task_tool();
        let _ = crate::tools::release_recovery::register_task_tools();
        let failed = ExecuteCommandResponse {
            operation: "draft_system_email".to_string(),
            status: CommandStatus::Failed,
            message: "untrusted raw Mail failure".to_string(),
            metrics: None,
            claims: vec!["CLAIM mail_draft_verified=false".to_string()],
            verified: false,
            model_used: None,
        };
        for operation in [
            "draft_system_email",
            "draft_decision_pack_email",
            "draft_release_recovery_email",
        ] {
            let raw = registered_task_result_failure(operation, &failed)
                .expect("failed Mail response must enter typed task-error handling");
            assert!(!raw.contains("untrusted raw Mail failure"));
            let normalized =
                crate::tools::task_tool_runtime::normalize_agent_error(operation, &raw);
            let parsed = crate::tools::task_tool_runtime::parse_agent_error(&normalized).unwrap();
            assert_eq!(parsed.code, "mail_draft_result_unverified");
            assert_ne!(parsed.boundary, "ToolRegistry");
            assert!(!parsed.changed_state_verified);
        }
    }

    #[test]
    fn typed_mail_change_truth_selects_resume_or_review_without_tool_registry_fallback() {
        let _ = crate::tools::decision_pack_mail::register_task_tool();
        let cases = [
            (
                "mail_automation_permission_required",
                serde_json::json!(false),
                crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved,
                "checkpoint_saved",
                "resume_same_execution",
                true,
            ),
            (
                "mail_draft_creation_failed_cleanly",
                serde_json::json!(false),
                crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved,
                "checkpoint_saved",
                "resume_same_execution",
                true,
            ),
            (
                "mail_draft_review_required",
                serde_json::json!("external_changes"),
                crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved,
                "external_changes",
                "review_external_changes",
                false,
            ),
            (
                "mail_draft_result_unverified",
                serde_json::Value::Null,
                crate::tools::task_tool_runtime::TaskToolChangedState::ExternalChanges,
                "external_changes",
                "review_external_changes",
                false,
            ),
        ];
        for (
            code,
            failed_step_change,
            completed_state,
            expected_change,
            expected_action,
            expected_recoverable,
        ) in cases
        {
            let raw = serde_json::json!({
                "taskToolError": {
                    "code": code,
                    "message": "Mail stopped at a verified boundary.",
                    "context": {
                        "failurePhase": "preflight",
                        "changedState": failed_step_change,
                    },
                }
            })
            .to_string();
            let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
                "draft_decision_pack_email",
                &raw,
            );
            let error = AgenticLoopError {
                code: "decision_pack_mail_draft_failed",
                boundary: "DecisionPackMailDraft",
                message: normalized,
                mlc_path: None,
            };
            let receipt =
                RecoveryReceipt::from_error("execution-mail", &plan(), &error, completed_state);
            let json: serde_json::Value =
                serde_json::from_str(&receipt.to_json().unwrap()).unwrap();
            assert_eq!(json["code"], code);
            assert_ne!(json["boundary"], "ToolRegistry");
            assert_eq!(json["changedState"], expected_change);
            assert_eq!(json["recoveryAction"], expected_action);
            assert_eq!(json["recoverable"], expected_recoverable);
        }
    }

    #[test]
    fn action_invocation_statuses_fail_closed_after_effectful_boundary() {
        for status in [
            ACTION_PREPARED_EFFECTFUL,
            ACTION_PREPARED_READ_ONLY,
            ACTION_STARTED_READ_ONLY,
            ACTION_UNVERIFIED_READ_ONLY,
            ACTION_SENSOR_READ_ONLY,
        ] {
            assert!(automatic_replay_safe_action_status(status), "{status}");
        }
        for status in [
            ACTION_STARTED_EFFECTFUL,
            ACTION_UNVERIFIED_EFFECTFUL,
            ACTION_SENSOR_EFFECTFUL,
            ACTION_FAILED_UNCHANGED_EFFECTFUL,
            ACTION_FAILED_UNCHANGED_READ_ONLY,
            "running",
            "blocked",
            "failed",
            "recoverable",
            "completed",
        ] {
            assert!(!automatic_replay_safe_action_status(status), "{status}");
        }
    }

    #[test]
    fn durable_recovery_state_detects_uncheckpointed_effectful_invocation() {
        let temp_dir = std::env::temp_dir().join(format!(
            "oomu-recovery-effect-boundary-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let persistence =
            PersistenceEngine::initialize_at(temp_dir.join("recovery.sqlite")).unwrap();
        let mut action_plan = plan();
        action_plan.steps.push(Step {
            step: "Write the approved output".to_string(),
            tool: Tool::FileWrite {
                path: "/tmp/recovery-output.txt".to_string(),
                content: "verified".to_string(),
            },
            risk_level: RiskLevel::High,
        });
        let plan_json = serialize_plan_for_persistence(&action_plan).unwrap();
        let connection = persistence.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO plan_generation_states
                 (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
                 VALUES (?1,?2,0,'running','running',1)",
                rusqlite::params![action_plan.id, plan_json],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                 VALUES (?1,'file_write','{}',NULL,?2,1)",
                rusqlite::params![action_plan.id, ACTION_STARTED_EFFECTFUL],
            )
            .unwrap();
        drop(connection);

        assert_eq!(
            durable_recovery_state(&persistence, &action_plan),
            crate::tools::task_tool_runtime::TaskToolChangedState::ExternalChanges
        );
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn verified_effectful_checkpoint_resumes_after_the_completed_step() {
        let temp_dir = std::env::temp_dir().join(format!(
            "oomu-recovery-effect-checkpoint-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let persistence =
            PersistenceEngine::initialize_at(temp_dir.join("recovery.sqlite")).unwrap();
        let mut action_plan = plan();
        action_plan.steps.extend([
            Step {
                step: "Create the approved decision pack".to_string(),
                tool: Tool::RegisteredTaskTool(
                    crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(
                        "create_decision_pack",
                        serde_json::json!({}),
                    ),
                ),
                risk_level: RiskLevel::High,
            },
            Step {
                step: "Create the conflict-free calendar event".to_string(),
                tool: Tool::RegisteredTaskTool(
                    crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(
                        "create_conflict_free_calendar_event",
                        serde_json::json!({}),
                    ),
                ),
                risk_level: RiskLevel::High,
            },
        ]);
        let plan_json = serialize_plan_for_persistence(&action_plan).unwrap();
        let output = serde_json::to_string(&ExecuteCommandResponse {
            operation: "create_decision_pack".to_string(),
            status: CommandStatus::Completed,
            message: "Four files were verified and published.".to_string(),
            metrics: None,
            claims: vec!["CLAIM decision_pack_file_verified=true".to_string()],
            verified: true,
            model_used: None,
        })
        .unwrap();
        let connection = persistence.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO plan_generation_states
                 (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
                 VALUES (?1,?2,1,'checkpointed','checkpointed',1)",
                rusqlite::params![action_plan.id, plan_json],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                 VALUES (?1,'create_decision_pack','{}',?2,'completed',1)",
                rusqlite::params![action_plan.id, output],
            )
            .unwrap();
        drop(connection);

        assert_eq!(
            durable_recovery_state(&persistence, &action_plan),
            crate::tools::task_tool_runtime::TaskToolChangedState::CheckpointSaved
        );
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    async fn typed_prepublication_failure_persists_retry_safe_sqlite_evidence() {
        crate::tools::task_tool_runtime::register_decision_pack_recovery_test_fixture();
        let temp_dir = std::env::temp_dir().join(format!(
            "oomu-recovery-verified-unchanged-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let persistence =
            PersistenceEngine::initialize_at(temp_dir.join("recovery.sqlite")).unwrap();
        let mut action_plan = plan();
        action_plan.steps.push(Step {
            step: "Create the approved decision pack".to_string(),
            tool: Tool::RegisteredTaskTool(
                crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(
                    "create_decision_pack",
                    serde_json::json!({}),
                ),
            ),
            risk_level: RiskLevel::High,
        });
        let plan_json = serialize_plan_for_persistence(&action_plan).unwrap();
        let connection = persistence.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO plan_generation_states
                 (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
                 VALUES (?1,?2,0,'running','running',1)",
                rusqlite::params![action_plan.id, plan_json],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                 VALUES (?1,'create_decision_pack','{}',NULL,?2,1)",
                rusqlite::params![action_plan.id, ACTION_STARTED_EFFECTFUL],
            )
            .unwrap();
        let action_id = connection.last_insert_rowid();
        drop(connection);

        let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
            "create_decision_pack",
            r#"{"taskToolError":{"code":"decision_pack_research_evidence_unavailable","message":"Recent official freight evidence was not verified.","context":{"subject":"freight","attemptCount":3,"pageCount":4,"verifiedInputCount":2,"changedState":false}}}"#,
        );
        let error = AgenticLoopError {
            code: "decision_pack_creation_failed",
            boundary: "DecisionPack",
            message: normalized,
            mlc_path: None,
        };
        record_failed_agent_action(
            &persistence,
            action_id,
            "create_decision_pack",
            &error,
            true,
        )
        .await
        .unwrap();

        let connection = persistence.open_connection().unwrap();
        let (status, output): (String, String) = connection
            .query_row(
                "SELECT status,output FROM actions WHERE id=?1",
                rusqlite::params![action_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        drop(connection);
        assert_eq!(status, ACTION_FAILED_UNCHANGED_EFFECTFUL);
        assert!(verified_unchanged_action_receipt(&status, Some(&output)));
        assert!(!persistence
            .has_uncertain_agent_action_effect(&action_plan.id)
            .unwrap());
        let receipt = RecoveryReceipt::from_error(
            "execution-retry-safe",
            &action_plan,
            &error,
            durable_recovery_state(&persistence, &action_plan),
        );
        let json: serde_json::Value = serde_json::from_str(&receipt.to_json().unwrap()).unwrap();
        assert_eq!(json["changedState"], "none");
        assert_eq!(json["recoveryAction"], "resume_same_execution");
        assert_eq!(json["recoverable"], true);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    async fn missing_typed_no_change_evidence_remains_fail_closed() {
        crate::tools::task_tool_runtime::register_decision_pack_recovery_test_fixture();
        let temp_dir = std::env::temp_dir().join(format!(
            "oomu-recovery-unknown-change-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let persistence =
            PersistenceEngine::initialize_at(temp_dir.join("recovery.sqlite")).unwrap();
        let connection = persistence.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                 VALUES ('plan-unknown','create_decision_pack','{}',NULL,?1,1)",
                rusqlite::params![ACTION_STARTED_EFFECTFUL],
            )
            .unwrap();
        let action_id = connection.last_insert_rowid();
        drop(connection);
        let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
            "create_decision_pack",
            r#"{"taskToolError":{"code":"decision_pack_research_evidence_unavailable","message":"Research stopped.","context":{"subject":"freight"}}}"#,
        );
        let error = AgenticLoopError {
            code: "decision_pack_creation_failed",
            boundary: "DecisionPack",
            message: normalized,
            mlc_path: None,
        };
        record_failed_agent_action(
            &persistence,
            action_id,
            "create_decision_pack",
            &error,
            true,
        )
        .await
        .unwrap();
        let connection = persistence.open_connection().unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM actions WHERE id=?1",
                rusqlite::params![action_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, ACTION_UNVERIFIED_EFFECTFUL);
        assert!(persistence
            .has_uncertain_agent_action_effect("plan-unknown")
            .unwrap());
        drop(connection);
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
