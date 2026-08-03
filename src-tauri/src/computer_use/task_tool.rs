use super::{
    commands::review_and_execute_app_control_action_core,
    contracts::{
        AppControlControl, ControlAppControlSessionRequest, DesktopSemanticAction,
        ExecuteDesktopActionRequest, ExpectedOutcomeKind, StartAppControlSession,
    },
    manager::AppControlManager,
    state::valid_bundle_id,
};
use crate::{
    shield_gate::{CommandStatus, ExecuteCommandResponse, ShieldApprovalManager},
    tools::task_tool_runtime::{
        TaskToolExecutionContext, TaskToolFuture, TaskToolRegistration, TaskToolValidation,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager as _;

const MAX_TASK_TOOL_ARGUMENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "phase",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum AppControlTaskToolRequest {
    GrantFile,
    Start {
        application_id: String,
        #[serde(default)]
        file_grant_ids: Vec<String>,
    },
    Observe {
        session_id: String,
    },
    Execute {
        session_id: String,
        observation_revision: u64,
        action: DesktopSemanticAction,
        expected_outcome: ExpectedOutcomeKind,
    },
    Stop {
        session_id: String,
    },
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: "app_control",
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: Some(planner_context),
        schema: task_tool_schema,
        metadata: crate::tools::task_tool_runtime::TaskToolMetadata {
            description: "Control one qualified desktop application through fresh observations, semantic actions, and exact Shield review.",
            risk_tier: crate::tools::task_tool_runtime::TaskToolRiskTier::SystemExec,
            approval_tier: crate::tools::task_tool_runtime::TaskToolApprovalTier::Background,
            agent_error_code: "app_control_tool_failed",
            agent_error_boundary: "AppControl",
            execution_path: "The native app_control tool ran through its Task-bound guarded lifecycle.",
        },
    })
}

pub(crate) fn task_tool_schema() -> Value {
    let reference = json!({"type":"string","pattern":"^appref_[0-9a-f]{48}$"});
    let file_grant = json!({"type":"string","pattern":"^appfile_[0-9a-f]{48}$"});
    let action = json!({
        "oneOf": [
            closed_object(json!({"kind":{"const":"focus"},"reference":reference.clone()}), &["kind","reference"]),
            closed_object(json!({"kind":{"const":"press"},"reference":reference.clone()}), &["kind","reference"]),
            closed_object(json!({"kind":{"const":"select"},"reference":reference.clone(),"value":{"type":"string","minLength":1,"maxLength":32767}}), &["kind","reference","value"]),
            closed_object(json!({"kind":{"const":"type_text"},"reference":reference.clone(),"text":{"type":"string","maxLength":32767}}), &["kind","reference","text"]),
            closed_object(json!({"kind":{"const":"invoke_menu"},"command":{"type":"string","enum":["save","save_as","new_window","close_window"]}}), &["kind","command"]),
            closed_object(json!({"kind":{"const":"scroll"},"reference":reference.clone(),"amount":{"type":"integer","minimum":-4000,"maximum":4000,"not":{"const":0}}}), &["kind","amount"]),
            closed_object(json!({"kind":{"const":"drag_drop"},"source":reference.clone(),"destination":reference.clone()}), &["kind","source","destination"]),
            closed_object(json!({"kind":{"const":"choose_file"},"reference":reference,"fileGrantId":file_grant.clone()}), &["kind","reference","fileGrantId"]),
            closed_object(json!({"kind":{"const":"apple_event"},"command":{"const":"activate_application"}}), &["kind","command"])
        ]
    });
    json!({
        "oneOf": [
            closed_object(
                json!({"phase":{"const":"grant_file"}}),
                &["phase"]
            ),
            closed_object(
                json!({
                    "phase":{"const":"start"},
                    "applicationId":{"type":"string","minLength":3,"maxLength":255},
                    "fileGrantIds":{"type":"array","maxItems":16,"uniqueItems":true,"items":file_grant}
                }),
                &["phase","applicationId"]
            ),
            closed_object(
                json!({
                    "phase":{"const":"observe"},
                    "sessionId":{"type":"string","pattern":"^appcontrol_[0-9a-f]{48}$"}
                }),
                &["phase","sessionId"]
            ),
            closed_object(
                json!({
                    "phase":{"const":"execute"},
                    "sessionId":{"type":"string","pattern":"^appcontrol_[0-9a-f]{48}$"},
                    "observationRevision":{"type":"integer","minimum":1},
                    "action":action,
                    "expectedOutcome":{"type":"string","enum":["no_change","element_value","element_state","window_state","application_state"]}
                }),
                &["phase","sessionId","observationRevision","action","expectedOutcome"]
            ),
            closed_object(
                json!({
                    "phase":{"const":"stop"},
                    "sessionId":{"type":"string","pattern":"^appcontrol_[0-9a-f]{48}$"}
                }),
                &["phase","sessionId"]
            )
        ]
    })
}

fn closed_object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type":"object",
        "properties":properties,
        "required":required,
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    if serde_json::to_vec(&arguments)
        .map_err(|_| "app_control_arguments_invalid".to_string())?
        .len()
        > MAX_TASK_TOOL_ARGUMENT_BYTES
    {
        return Err("app_control_arguments_too_large".to_string());
    }
    let request = serde_json::from_value::<AppControlTaskToolRequest>(arguments)
        .map_err(|_| "app_control_arguments_invalid".to_string())?;
    validate_request(&request)?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request)
            .map_err(|_| "app_control_arguments_invalid".to_string())?,
        // The execution phase performs its own exact, structured Shield review.
        // Marking this bridge effectful would create a second generic prompt.
        potentially_effectful: false,
    })
}

fn validate_request(request: &AppControlTaskToolRequest) -> Result<(), String> {
    match request {
        AppControlTaskToolRequest::GrantFile => {}
        AppControlTaskToolRequest::Start {
            application_id,
            file_grant_ids,
        } => {
            if !valid_bundle_id(application_id)
                || file_grant_ids.len() > 16
                || file_grant_ids
                    .iter()
                    .any(|grant| !valid_opaque(grant, "appfile"))
            {
                return Err("app_control_application_invalid".to_string());
            }
        }
        AppControlTaskToolRequest::Observe { session_id }
        | AppControlTaskToolRequest::Stop { session_id } => {
            if !valid_session_id(session_id) {
                return Err("app_control_session_invalid".to_string());
            }
        }
        AppControlTaskToolRequest::Execute {
            session_id,
            action,
            expected_outcome,
            ..
        } => {
            if !valid_session_id(session_id)
                || action
                    .references()
                    .iter()
                    .any(|reference| !valid_opaque(reference, "appref"))
                || matches!(
                    action,
                    DesktopSemanticAction::ChooseFile { file_grant_id, .. }
                        if !valid_opaque(file_grant_id, "appfile")
                )
                || !planner_action_qualified(action, *expected_outcome)
            {
                return Err("app_control_execute_invalid".to_string());
            }
        }
    }
    Ok(())
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<AppControlTaskToolRequest>(arguments)
            .map_err(|_| "app_control_arguments_invalid".to_string())?;
        validate_request(&request)?;
        execute_task_phase(context, request).await
    })
}

async fn execute_task_phase(
    context: TaskToolExecutionContext<'_>,
    request: AppControlTaskToolRequest,
) -> Result<ExecuteCommandResponse, String> {
    let execution_id = context
        .execution_id
        .ok_or_else(|| "app_control_requires_agent_task".to_string())?;
    let task = crate::tasks::require_agent_runtime_task(context.persistence, execution_id)?;
    let project_id = task
        .project_id
        .clone()
        .ok_or_else(|| "app_control_requires_project_task".to_string())?;
    let app = context
        .app
        .ok_or_else(|| "app_control_requires_desktop_runtime".to_string())?;
    let manager = app
        .try_state::<AppControlManager>()
        .ok_or_else(|| "app_control_runtime_unavailable".to_string())?;

    match request {
        AppControlTaskToolRequest::GrantFile => {
            let selected = rfd::AsyncFileDialog::new().pick_file().await;
            let Some(selected) = selected else {
                return response(
                    json!({ "phase": "file_grant_cancelled" }),
                    "CLAIM app_control_file_grant_cancelled=true".to_string(),
                );
            };
            let grant = manager
                .grant_selected_file(
                    &project_id,
                    &task.task_run_id,
                    selected.path().to_path_buf(),
                )
                .map_err(|error| error.message)?;
            response(
                json!({ "phase": "file_granted", "grant": grant }),
                "CLAIM app_control_file_grant_issued=true".to_string(),
            )
        }
        AppControlTaskToolRequest::Start {
            application_id,
            file_grant_ids,
        } => {
            let session = manager
                .start_session(StartAppControlSession {
                    project_id,
                    task_run_id: task.task_run_id,
                    approved_bundle_ids: vec![application_id],
                    scoped_file_roots: Vec::new(),
                    file_grant_ids,
                })
                .map_err(|error| error.message)?;
            response(
                json!({ "phase": "started", "session": session }),
                "CLAIM app_control_session_started=true".to_string(),
            )
        }
        AppControlTaskToolRequest::Observe { session_id } => {
            observe_with_receipt(
                &context,
                execution_id,
                manager.inner(),
                session_id,
                task.task_run_id,
            )
            .await
        }
        AppControlTaskToolRequest::Execute {
            session_id,
            observation_revision,
            action,
            expected_outcome,
        } => {
            execute_action_phase(
                &context,
                execution_id,
                app,
                manager.inner(),
                task.task_run_id,
                session_id,
                observation_revision,
                action,
                expected_outcome,
            )
            .await
        }
        AppControlTaskToolRequest::Stop { session_id } => {
            let session = manager
                .control(ControlAppControlSessionRequest {
                    session_id,
                    task_run_id: task.task_run_id,
                    control: AppControlControl::Stop,
                })
                .map_err(|error| error.message)?;
            response(
                json!({ "phase": "stopped", "session": session }),
                "CLAIM app_control_session_stopped=true".to_string(),
            )
        }
    }
}

async fn observe_with_receipt(
    context: &TaskToolExecutionContext<'_>,
    execution_id: &str,
    manager: &AppControlManager,
    session_id: String,
    task_run_id: String,
) -> Result<ExecuteCommandResponse, String> {
    use crate::tools::native_operation_receipt::{
        AppleCapability, NativeActionClass, NativeOperationAttempt, NativePostconditionEvidence,
    };
    let attempt = NativeOperationAttempt::begin_for_execution(
        AppleCapability::Accessibility,
        NativeActionClass::Observe,
        false,
        crate::foundation::digest::sha256_hex(session_id.as_bytes()),
        context.persistence,
        execution_id,
    )
    .await;
    let observation = match manager.observe(&session_id, &task_run_id) {
        Ok(observation) => observation,
        Err(error) => {
            if let Some(attempt) = attempt {
                attempt
                    .finish(NativePostconditionEvidence {
                        evidence_kind: "accessibility_observation_error",
                        operation_succeeded: false,
                        verified: false,
                        bounded_count: None,
                        truncated: None,
                        native_result_code: Some(format!("{:?}", error.code).to_ascii_lowercase()),
                        durable_operation_binding: None,
                        capture_proof: None,
                    })
                    .await;
            }
            return Err(error.message);
        }
    };
    if let Some(attempt) = attempt {
        attempt
            .finish(NativePostconditionEvidence {
                evidence_kind: "verified_accessibility_observation",
                operation_succeeded: true,
                verified: observation.observation_hash.len() == 64,
                bounded_count: Some(observation.elements.len() as u64),
                truncated: None,
                native_result_code: Some("observed".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            })
            .await;
    }
    let claim = format!(
        "CLAIM app_control_observation_sha256={} generation={} revision={}",
        observation.observation_hash, observation.generation, observation.revision
    );
    response(
        json!({ "phase": "observed", "observation": observation }),
        claim,
    )
}

#[allow(clippy::too_many_arguments)]
async fn execute_action_phase(
    context: &TaskToolExecutionContext<'_>,
    execution_id: &str,
    app: &tauri::AppHandle,
    manager: &AppControlManager,
    task_run_id: String,
    session_id: String,
    observation_revision: u64,
    action: DesktopSemanticAction,
    expected_outcome: ExpectedOutcomeKind,
) -> Result<ExecuteCommandResponse, String> {
    use crate::tools::native_operation_receipt::{
        AppleCapability, NativeActionClass, NativeOperationAttempt, NativePostconditionEvidence,
    };
    let approvals = app
        .try_state::<ShieldApprovalManager>()
        .ok_or_else(|| "app_control_approval_runtime_unavailable".to_string())?;
    let desktop_request = ExecuteDesktopActionRequest {
        session_id,
        task_run_id,
        observation_revision,
        action,
        expected_outcome,
    };
    let authority = manager
        .authority_request_for(&desktop_request)
        .map_err(|error| error.message)?;
    let native_attempt = NativeOperationAttempt::begin_for_execution(
        AppleCapability::ScreenControl,
        NativeActionClass::Control,
        false,
        authority.action_arguments_hash.clone(),
        context.persistence,
        execution_id,
    )
    .await;
    let outcome = match review_and_execute_app_control_action_core(
        desktop_request,
        manager,
        approvals.inner(),
        app,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            finish_failed_app_control(native_attempt).await;
            return Err(error.message);
        }
    };
    if let Some(attempt) = native_attempt {
        attempt
            .action_was_approved()
            .finish(NativePostconditionEvidence {
                evidence_kind: "verified_desktop_outcome",
                operation_succeeded: true,
                verified: true,
                bounded_count: Some(1),
                truncated: None,
                native_result_code: Some("completed".to_string()),
                durable_operation_binding: Some(crate::foundation::digest::sha256_hex(
                    outcome.receipt.receipt_id.as_bytes(),
                )),
                capture_proof: None,
            })
            .await;
    }
    let claim = format!(
        "CLAIM app_control_receipt={} postcondition_sha256={} status={:?}",
        outcome.receipt.receipt_id, outcome.receipt.postcondition_hash, outcome.receipt.status
    );
    response(json!({ "phase": "executed", "outcome": outcome }), claim)
}

async fn finish_failed_app_control(
    attempt: Option<crate::tools::native_operation_receipt::NativeOperationAttempt>,
) {
    use crate::tools::native_operation_receipt::NativePostconditionEvidence;
    if let Some(attempt) = attempt {
        attempt
            .finish(NativePostconditionEvidence {
                evidence_kind: "verified_desktop_outcome",
                operation_succeeded: false,
                verified: false,
                bounded_count: None,
                truncated: None,
                native_result_code: Some("app_control_failed".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            })
            .await;
    }
}

fn response(message: Value, claim: String) -> Result<ExecuteCommandResponse, String> {
    Ok(ExecuteCommandResponse {
        operation: "app_control".to_string(),
        status: CommandStatus::Completed,
        message: serde_json::to_string(&message)
            .map_err(|_| "app_control_result_invalid".to_string())?,
        metrics: None,
        claims: vec![claim],
        verified: true,
        model_used: None,
    })
}

fn planner_context(
    persistence: &crate::db::PersistenceEngine,
    session_id: &str,
) -> Result<Option<String>, String> {
    if persistence
        .project_inference_context_for_session(session_id)?
        .is_none()
    {
        return Ok(None);
    }
    Ok(Some(
        "Native app control is available through app_control. For file selection, use grant_file before start and pass its opaque grant to start. Then use start, observe, and execute with only the returned session, revision, and references. Never invent references or paths. Use stop when finished. Browser work must use browser automation instead."
            .to_string(),
    ))
}

fn valid_session_id(value: &str) -> bool {
    valid_opaque(value, "appcontrol")
}

fn valid_opaque(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(&format!("{prefix}_"))
        .is_some_and(|suffix| {
            suffix.len() == 48
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn planner_action_qualified(action: &DesktopSemanticAction, expected: ExpectedOutcomeKind) -> bool {
    use super::contracts::{QualifiedAppleEvent, QualifiedMenuCommand};
    match (action, expected) {
        (DesktopSemanticAction::Focus { .. }, ExpectedOutcomeKind::ElementState) => true,
        (
            DesktopSemanticAction::Select { .. } | DesktopSemanticAction::TypeText { .. },
            ExpectedOutcomeKind::ElementValue,
        ) => true,
        (
            DesktopSemanticAction::Press { .. },
            ExpectedOutcomeKind::ElementState
            | ExpectedOutcomeKind::WindowState
            | ExpectedOutcomeKind::ApplicationState,
        ) => true,
        (DesktopSemanticAction::Scroll { amount, .. }, ExpectedOutcomeKind::ApplicationState) => {
            *amount != 0 && amount.unsigned_abs() <= 4_000
        }
        (
            DesktopSemanticAction::InvokeMenu { command },
            ExpectedOutcomeKind::WindowState | ExpectedOutcomeKind::ApplicationState,
        ) => !matches!(command, QualifiedMenuCommand::Export),
        (DesktopSemanticAction::DragDrop { .. }, ExpectedOutcomeKind::ApplicationState) => true,
        (DesktopSemanticAction::ChooseFile { .. }, ExpectedOutcomeKind::WindowState) => true,
        (
            DesktopSemanticAction::AppleEvent {
                command: QualifiedAppleEvent::ActivateApplication,
            },
            ExpectedOutcomeKind::ApplicationState,
        ) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_tool_contract_is_bounded_and_multiphase() {
        for request in [
            json!({"phase":"grant_file"}),
            json!({"phase":"start","applicationId":"com.apple.mail","fileGrantIds":[]}),
            json!({"phase":"observe","sessionId":format!("appcontrol_{}", "0".repeat(48))}),
            json!({"phase":"stop","sessionId":format!("appcontrol_{}", "0".repeat(48))}),
        ] {
            assert!(validate_registration(request).is_ok());
        }
        assert!(validate_registration(json!({
            "phase":"execute",
            "sessionId":format!("appcontrol_{}", "0".repeat(48)),
            "observationRevision":1,
            "action":{"kind":"run_script","script":"unsafe"},
            "expectedOutcome":"application_state"
        }))
        .is_err());
        assert!(validate_registration(json!({
            "phase":"execute",
            "sessionId":format!("appcontrol_{}", "0".repeat(48)),
            "observationRevision":1,
            "action":{"kind":"choose_file","reference":format!("appref_{}", "0".repeat(48)),"path":"/tmp/guessed"},
            "expectedOutcome":"window_state"
        }))
        .is_err());
        assert!(validate_registration(json!({
            "phase":"observe",
            "sessionId":"invented"
        }))
        .is_err());
    }
}
