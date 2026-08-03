//! Neutral callback bridge for Project-bound native Task tools.
use crate::{
    db::PersistenceEngine,
    shield_gate::{AuthorizedActions, ExecuteCommandResponse, RequestedAction},
    sovereign_identity::SovereignIdentity,
    tools::task_tool_error,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{OnceLock, RwLock},
};

#[path = "task_tool_runtime/native_receipt.rs"]
mod native_receipt;

pub(crate) use task_tool_error::ChangedState as TaskToolChangedState;

pub(crate) const TASK_RUN_TIMESTAMP_TOKEN: &str = "<YYYY-MM-DD_HH-mm>";

pub(crate) type TaskToolFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ExecuteCommandResponse, String>> + Send + 'a>>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedTaskToolRequest {
    pub operation: String,
    pub arguments: Value,
}

impl PlannedTaskToolRequest {
    pub(crate) fn new(operation: impl Into<String>, arguments: Value) -> Self {
        Self {
            operation: operation.into(),
            arguments,
        }
    }

    pub(crate) fn potentially_effectful(&self) -> bool {
        validate_if_registered(&self.operation, self.arguments.clone())
            .and_then(Result::ok)
            .map(|validation| validation.potentially_effectful)
            .unwrap_or(true)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ValidatedTaskToolRequest {
    pub operation: &'static str,
    pub arguments: Value,
    pub potentially_effectful: bool,
}

impl ValidatedTaskToolRequest {
    pub(crate) fn potentially_effectful(&self) -> bool {
        self.potentially_effectful
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TaskToolValidation {
    pub arguments: Value,
    pub potentially_effectful: bool,
}

pub(crate) struct TaskToolExecutionContext<'a> {
    pub persistence: &'a PersistenceEngine,
    pub identity: &'a SovereignIdentity,
    pub app: Option<&'a tauri::AppHandle>,
    pub execution_id: Option<&'a str>,
    pub plan_id: Option<&'a str>,
    pub objective: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub model_route: Option<&'a crate::agentic_loop::ModelRouteDecision>,
}

pub(crate) type ValidateTaskTool = fn(Value) -> Result<TaskToolValidation, String>;
pub(crate) type ResolveTaskTool =
    fn(&PersistenceEngine, Option<&str>, Value, &[ExecuteCommandResponse]) -> Result<Value, String>;
pub(crate) type ExecuteTaskTool =
    for<'a> fn(TaskToolExecutionContext<'a>, Value) -> TaskToolFuture<'a>;
pub(crate) type PlannerContext = fn(&PersistenceEngine, &str) -> Result<Option<String>, String>;
pub(crate) type TaskToolSchema = fn() -> Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskToolRiskTier {
    ReadOnly,
    FileRead,
    FileWrite,
    SystemExec,
    Network,
}

impl TaskToolRiskTier {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::FileRead => "file_read",
            Self::FileWrite => "file_write",
            Self::SystemExec => "system_exec",
            Self::Network => "network",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskToolApprovalTier {
    Background,
    Visual,
    Explicit,
}

#[derive(Clone, Copy)]
pub(crate) struct TaskToolMetadata {
    pub description: &'static str,
    pub risk_tier: TaskToolRiskTier,
    pub approval_tier: TaskToolApprovalTier,
    pub agent_error_code: &'static str,
    pub agent_error_boundary: &'static str,
    pub execution_path: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) struct TaskToolRegistration {
    pub operation: &'static str,
    pub validate: ValidateTaskTool,
    pub validate_resolved: ValidateTaskTool,
    pub resolve: ResolveTaskTool,
    pub execute: ExecuteTaskTool,
    pub planner_context: Option<PlannerContext>,
    pub schema: TaskToolSchema,
    pub metadata: TaskToolMetadata,
}

fn registrations() -> &'static RwLock<BTreeMap<&'static str, TaskToolRegistration>> {
    static REGISTRATIONS: OnceLock<RwLock<BTreeMap<&'static str, TaskToolRegistration>>> =
        OnceLock::new();
    REGISTRATIONS.get_or_init(|| RwLock::new(BTreeMap::new()))
}

pub(crate) fn register(registration: TaskToolRegistration) -> Result<(), String> {
    if registration.operation.is_empty()
        || registration.operation.len() > 64
        || !registration
            .operation
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err("task_tool_registration_name_invalid".to_string());
    }
    let mut registry = registrations()
        .write()
        .map_err(|_| "task_tool_registry_unavailable".to_string())?;
    if registry.contains_key(registration.operation) {
        return Err("task_tool_registration_duplicate".to_string());
    }
    registry.insert(registration.operation, registration);
    Ok(())
}

fn registration(operation: &str) -> Result<TaskToolRegistration, String> {
    registrations()
        .read()
        .map_err(|_| "task_tool_registry_unavailable".to_string())?
        .get(operation)
        .copied()
        .ok_or_else(|| "task_tool_not_registered".to_string())
}

pub(crate) fn registered_operations() -> Vec<&'static str> {
    registrations()
        .read()
        .map(|registry| registry.keys().copied().collect())
        .unwrap_or_default()
}

pub(crate) fn is_registered(operation: &str) -> bool {
    registration(&normalize_operation(operation)).is_ok()
}

pub(crate) fn validate(
    operation: &str,
    arguments: Value,
) -> Result<ValidatedTaskToolRequest, String> {
    let registration = registration(&normalize_operation(operation))?;
    let validation = (registration.validate)(arguments)?;
    Ok(ValidatedTaskToolRequest {
        operation: registration.operation,
        arguments: validation.arguments,
        potentially_effectful: validation.potentially_effectful,
    })
}

pub(crate) fn validate_if_registered(
    operation: &str,
    arguments: Value,
) -> Option<Result<TaskToolValidation, String>> {
    let registration = registration(&normalize_operation(operation)).ok()?;
    Some((registration.validate)(arguments))
}

pub(crate) fn validate_generated_tool(
    kind: &str,
    tool: &Map<String, Value>,
) -> Option<Result<(), String>> {
    let mut arguments = tool.clone();
    arguments.remove("kind");
    validate_if_registered(&normalize_operation(kind), Value::Object(arguments))
        .map(|result| result.map(|_| ()))
}

pub(crate) fn validated_generated_arguments(operation: &str, value: &Value) -> Option<Value> {
    let mut arguments = value.as_object()?.clone();
    arguments.remove("kind");
    let registration = registration(&normalize_operation(operation)).ok()?;
    (registration.validate)(Value::Object(arguments))
        .ok()
        .map(|validation| validation.arguments)
}

pub(crate) fn schema(operation: &str) -> Result<Value, String> {
    registration(&normalize_operation(operation)).map(|registration| (registration.schema)())
}

pub(crate) fn description(operation: &str) -> Result<&'static str, String> {
    registration(&normalize_operation(operation)).map(|value| value.metadata.description)
}

pub(crate) fn risk_tier(operation: &str) -> Result<TaskToolRiskTier, String> {
    registration(&normalize_operation(operation)).map(|value| value.metadata.risk_tier)
}

pub(crate) fn approval_tier(operation: &str) -> Option<TaskToolApprovalTier> {
    registration(&normalize_operation(operation))
        .ok()
        .map(|value| value.metadata.approval_tier)
}

pub(crate) fn requires_explicit_approval(operation: &str) -> bool {
    approval_tier(operation) == Some(TaskToolApprovalTier::Explicit)
}

pub(crate) fn requested_action(request: &PlannedTaskToolRequest) -> RequestedAction {
    let operation = normalize_operation(&request.operation);
    let path = match operation.as_str() {
        "read_project_file" => request.arguments.get("path").and_then(Value::as_str),
        "create_file" => request
            .arguments
            .pointer("/file/destinationPath")
            .and_then(Value::as_str),
        "create_decision_pack" => request
            .arguments
            .get("outputDirectory")
            .and_then(Value::as_str),
        "prepare_release_recovery_agenda" => {
            request.arguments.get("outputPath").and_then(Value::as_str)
        }
        "prepare_background_agent_comparison" | "prepare_milestone_constraint_recovery_plan" => {
            request.arguments.get("outputPath").and_then(Value::as_str)
        }
        _ => None,
    }
    .map(str::to_string);
    RequestedAction {
        kind: request.operation.to_string(),
        principal: None,
        path,
        content: serde_json::to_string(&request.arguments).ok(),
    }
}

pub(crate) fn requested_action_for_validated(
    request: &ValidatedTaskToolRequest,
) -> RequestedAction {
    requested_action(&PlannedTaskToolRequest::new(
        request.operation,
        request.arguments.clone(),
    ))
}

pub(crate) fn authorize(action: RequestedAction) -> Result<ValidatedTaskToolRequest, String> {
    if action.principal.is_some() {
        return Err("task_tool_action_envelope_invalid".to_string());
    }
    let operation = normalize_operation(&action.kind);
    let content = action
        .content
        .ok_or_else(|| "task_tool_arguments_required".to_string())?;
    let arguments = serde_json::from_str::<Value>(&content)
        .map_err(|_| "task_tool_arguments_invalid".to_string())?;
    let registration = registration(&operation)?;
    let validation = (registration.validate)(arguments.clone())
        .or_else(|_| (registration.validate_resolved)(arguments))?;
    let mut validated = ValidatedTaskToolRequest {
        operation: registration.operation,
        arguments: validation.arguments,
        potentially_effectful: validation.potentially_effectful,
    };
    if matches!(
        operation.as_str(),
        "read_project_file"
            | "create_file"
            | "create_decision_pack"
            | "prepare_release_recovery_agenda"
            | "prepare_background_agent_comparison"
            | "prepare_milestone_constraint_recovery_plan"
    ) {
        let expected_path = match operation.as_str() {
            "read_project_file" => validated.arguments.get("path").and_then(Value::as_str),
            "create_file" => validated
                .arguments
                .pointer("/file/destinationPath")
                .and_then(Value::as_str),
            "create_decision_pack" => validated
                .arguments
                .get("outputDirectory")
                .and_then(Value::as_str),
            "prepare_release_recovery_agenda" => validated
                .arguments
                .get("outputPath")
                .and_then(Value::as_str),
            "prepare_background_agent_comparison"
            | "prepare_milestone_constraint_recovery_plan" => validated
                .arguments
                .get("outputPath")
                .and_then(Value::as_str),
            _ => None,
        }
        .ok_or_else(|| "task_tool_action_envelope_invalid".to_string())?;
        if action.path.as_deref() != Some(expected_path) {
            return Err("task_tool_action_envelope_invalid".to_string());
        }
    } else if action.path.is_some() {
        return Err("task_tool_action_envelope_invalid".to_string());
    }
    if matches!(
        operation.as_str(),
        "prepare_background_agent_comparison" | "prepare_milestone_constraint_recovery_plan"
    ) {
        let arguments =
            super::evidence_artifacts::bind_authorized_arguments(&operation, validated.arguments)?;
        let validation = (registration.validate_resolved)(arguments)?;
        validated.arguments = validation.arguments;
        validated.potentially_effectful = validation.potentially_effectful;
    }
    Ok(validated)
}

pub(crate) fn resolve(
    persistence: &PersistenceEngine,
    execution_id: Option<&str>,
    request: ValidatedTaskToolRequest,
    outputs: &[ExecuteCommandResponse],
) -> Result<ValidatedTaskToolRequest, String> {
    let registration = registration(request.operation)?;
    let arguments = (registration.resolve)(persistence, execution_id, request.arguments, outputs)?;
    let validation = (registration.validate_resolved)(arguments)?;
    Ok(ValidatedTaskToolRequest {
        operation: request.operation,
        arguments: validation.arguments,
        potentially_effectful: validation.potentially_effectful,
    })
}

pub(crate) fn resolve_authorized_action(
    persistence: &PersistenceEngine,
    execution_id: Option<&str>,
    action: AuthorizedActions,
    planned: RequestedAction,
    outputs: &[ExecuteCommandResponse],
) -> Result<(AuthorizedActions, RequestedAction), String> {
    let wrap = |request: ValidatedTaskToolRequest| {
        let requested = requested_action_for_validated(&request);
        (AuthorizedActions::RegisteredTaskTool(request), requested)
    };
    match action {
        AuthorizedActions::RegisteredTaskTool(request) => {
            resolve(persistence, execution_id, request, outputs).map(wrap)
        }
        action => Ok((action, planned)),
    }
}

pub(crate) async fn execute(
    context: TaskToolExecutionContext<'_>,
    request: ValidatedTaskToolRequest,
) -> Result<ExecuteCommandResponse, String> {
    let registration = registration(request.operation)?;
    let receipt = native_receipt::begin(
        context.persistence,
        context.execution_id,
        request.operation,
        &request.arguments,
    )
    .await;
    let result = (registration.execute)(context, request.arguments).await;
    native_receipt::finish(receipt, &result).await;
    result
}

pub(crate) fn premise(request: &PlannedTaskToolRequest) -> String {
    let digest = serde_json::to_vec(&request.arguments)
        .map(|bytes| crate::foundation::digest::sha256_hex(&bytes))
        .unwrap_or_else(|_| "unavailable".to_string());
    let risk_tier = registration(&request.operation)
        .map(|registration| registration.metadata.risk_tier.as_str())
        .unwrap_or("unregistered");
    format!(
        "task_tool={} risk_tier={risk_tier} arguments_sha256={digest}",
        request.operation
    )
}

pub(crate) fn agent_error_metadata(operation: &str) -> (&'static str, &'static str) {
    registration(operation)
        .map(|value| {
            (
                value.metadata.agent_error_code,
                value.metadata.agent_error_boundary,
            )
        })
        .unwrap_or(("registered_task_tool_failed", "RegisteredTaskTool"))
}

pub(crate) fn normalize_agent_error(operation: &str, raw: &str) -> String {
    let registration = registration(operation).ok();
    let code = registration
        .map(|value| value.metadata.agent_error_code)
        .unwrap_or("registered_task_tool_failed");
    let boundary = registration
        .map(|value| value.metadata.agent_error_boundary)
        .unwrap_or("RegisteredTaskTool");
    let message = registration
        .map(|value| format!("{} could not finish safely.", value.metadata.description))
        .unwrap_or_else(|| "The registered task could not finish safely.".to_string());
    task_tool_error::encode(&task_tool_error::decode(raw, code, boundary, &message))
}

pub(crate) fn parse_agent_error(raw: &str) -> Option<task_tool_error::TaskToolAgentError> {
    task_tool_error::decode_normalized(raw)
}

pub(crate) fn parse_retry_safe_unchanged_error(
    operation: &str,
    raw: &str,
) -> Option<task_tool_error::TaskToolAgentError> {
    let registration = registration(operation).ok()?;
    let error = parse_agent_error(raw)?;
    let code_is_retry_safe = matches!(
        (registration.operation, error.code.as_str()),
        (
            "create_decision_pack",
            "decision_pack_research_network_unavailable"
                | "decision_pack_research_evidence_unavailable"
        ) | (
            "fetch_official_page",
            "network_unavailable"
                | "dns_resolution_failed"
                | "network_timeout"
                | "connection_failed"
        ) | (
            "create_conflict_free_calendar_event"
                | "create_system_calendar_event"
                | "create_release_recovery_calendar_event",
            "calendar_action_denied"
                | "calendar_not_found"
                | "calendar_name_ambiguous"
                | "calendar_read_only"
                | "calendar_availability_unsupported"
                | "calendar_permission_denied"
                | "calendar_permission_restricted"
                | "calendar_permission_write_only"
                | "calendar_permission_unavailable"
                | "calendar_authorization_timeout"
                | "calendar_agenda_binding_changed"
        ) | (
            "draft_system_email" | "draft_decision_pack_email" | "draft_release_recovery_email",
            "mail_automation_permission_required"
                | "mail_automation_timeout"
                | "mail_automation_unavailable"
                | "mail_draft_creation_failed_cleanly"
                | "mail_agenda_binding_changed"
        ) | (
            "prepare_background_agent_comparison" | "prepare_milestone_constraint_recovery_plan",
            "evidence_artifact_preparation_failed"
        )
    );
    (error.boundary == registration.metadata.agent_error_boundary
        && error.changed_state_verified
        && error.changed_state == TaskToolChangedState::None
        && code_is_retry_safe)
        .then_some(error)
}

pub(crate) fn agent_execution_path(operation: &str) -> String {
    registration(operation)
        .map(|value| value.metadata.execution_path)
        .unwrap_or("The registered Task tool completed through its bounded native lifecycle.")
        .to_string()
}

pub(crate) fn append_planner_context(
    persistence: &PersistenceEngine,
    session_id: Option<&str>,
    prompt: &mut String,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    let callbacks = registrations()
        .read()
        .map_err(|_| "task_tool_registry_unavailable".to_string())?
        .values()
        .filter_map(|registration| registration.planner_context)
        .collect::<Vec<_>>();
    for callback in callbacks {
        if let Some(context) = callback(persistence, session_id)? {
            prompt.push_str("\n\n");
            prompt.push_str(&context);
        }
    }
    Ok(())
}

pub(crate) fn identity_resolver(
    _persistence: &PersistenceEngine,
    _execution_id: Option<&str>,
    arguments: Value,
    _outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    Ok(arguments)
}

fn normalize_operation(value: &str) -> String {
    value.trim().replace('-', "_").to_ascii_lowercase()
}

#[cfg(test)]
pub(crate) fn register_app_control_test_fixture() {
    fn validate(arguments: Value) -> Result<TaskToolValidation, String> {
        let phase = arguments
            .get("phase")
            .and_then(Value::as_str)
            .ok_or_else(|| "app_control_phase_required".to_string())?;
        if !matches!(phase, "start" | "observe" | "execute" | "stop")
            || arguments.get("action").is_some_and(|action| {
                action.get("kind").and_then(Value::as_str) == Some("run_script")
                    || action.get("x").is_some()
                    || action.get("y").is_some()
            })
        {
            return Err("app_control_fixture_invalid".to_string());
        }
        Ok(TaskToolValidation {
            arguments,
            potentially_effectful: false,
        })
    }
    fn execute<'a>(
        _context: TaskToolExecutionContext<'a>,
        _arguments: Value,
    ) -> TaskToolFuture<'a> {
        Box::pin(async { Err("fixture_not_executed".to_string()) })
    }
    let registration = TaskToolRegistration {
        operation: "app_control",
        validate,
        validate_resolved: validate,
        resolve: identity_resolver,
        execute,
        planner_context: None,
        schema: || serde_json::json!({"type":"object"}),
        metadata: TaskToolMetadata {
            description: "Test-only app control fixture.",
            risk_tier: TaskToolRiskTier::SystemExec,
            approval_tier: TaskToolApprovalTier::Background,
            agent_error_code: "app_control_tool_failed",
            agent_error_boundary: "AppControl",
            execution_path: "Test-only app control path.",
        },
    };
    if register(registration).is_err() {
        assert!(schema("app_control").is_ok());
    }
}

#[cfg(test)]
pub(crate) fn register_decision_pack_recovery_test_fixture() {
    fn validate(arguments: Value) -> Result<TaskToolValidation, String> {
        Ok(TaskToolValidation {
            arguments,
            potentially_effectful: true,
        })
    }
    fn execute<'a>(
        _context: TaskToolExecutionContext<'a>,
        _arguments: Value,
    ) -> TaskToolFuture<'a> {
        Box::pin(async { Err("decision_pack_recovery_fixture_not_executed".to_string()) })
    }
    let registration = TaskToolRegistration {
        operation: "create_decision_pack",
        validate,
        validate_resolved: validate,
        resolve: identity_resolver,
        execute,
        planner_context: None,
        schema: || serde_json::json!({"type":"object"}),
        metadata: TaskToolMetadata {
            description: "Test-only decision-pack recovery fixture.",
            risk_tier: TaskToolRiskTier::Network,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "decision_pack_creation_failed",
            agent_error_boundary: "DecisionPack",
            execution_path: "Test-only decision-pack recovery path.",
        },
    };
    if register(registration).is_err() {
        assert!(schema("create_decision_pack").is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn validate_fixture(arguments: Value) -> Result<TaskToolValidation, String> {
        if arguments.get("value").and_then(Value::as_str) != Some("ready") {
            return Err("fixture_invalid".to_string());
        }
        Ok(TaskToolValidation {
            arguments,
            potentially_effectful: false,
        })
    }

    fn execute_fixture<'a>(
        _context: TaskToolExecutionContext<'a>,
        _arguments: Value,
    ) -> TaskToolFuture<'a> {
        Box::pin(async {
            Ok(ExecuteCommandResponse {
                operation: "fixture_tool".to_string(),
                status: crate::shield_gate::CommandStatus::Completed,
                message: "ready".to_string(),
                metrics: None,
                claims: vec!["CLAIM fixture=true".to_string()],
                verified: true,
                model_used: None,
            })
        })
    }

    #[test]
    fn registration_validation_is_fail_closed() {
        assert_eq!(
            validate("missing_tool", json!({})).unwrap_err(),
            "task_tool_not_registered"
        );
        register(TaskToolRegistration {
            operation: "fixture_tool",
            validate: validate_fixture,
            validate_resolved: validate_fixture,
            resolve: identity_resolver,
            execute: execute_fixture,
            planner_context: None,
            schema: || serde_json::json!({"type":"object"}),
            metadata: TaskToolMetadata {
                description: "Test-only registered Task tool.",
                risk_tier: TaskToolRiskTier::ReadOnly,
                approval_tier: TaskToolApprovalTier::Background,
                agent_error_code: "fixture_failed",
                agent_error_boundary: "Fixture",
                execution_path: "Test-only registered Task tool path.",
            },
        })
        .unwrap();
        assert!(validate("fixture_tool", json!({"value":"bad"})).is_err());
        assert_eq!(
            validate("fixture_tool", json!({"value":"ready"}))
                .unwrap()
                .arguments["value"],
            "ready"
        );
    }

    #[test]
    fn app_control_authorization_reaches_closed_registered_variant() {
        register_app_control_test_fixture();
        for phase in ["start", "observe", "execute", "stop"] {
            let planned =
                PlannedTaskToolRequest::new("app_control", serde_json::json!({"phase":phase}));
            let action = requested_action(&planned);
            assert!(matches!(
                authorize(action),
                Ok(ValidatedTaskToolRequest {
                    operation: "app_control",
                    ..
                })
            ));
        }
        assert!(authorize(RequestedAction {
            kind: "app_control".to_string(),
            principal: None,
            path: None,
            content: Some(
                serde_json::json!({
                    "phase":"execute",
                    "action":{"kind":"run_script","script":"unsafe"}
                })
                .to_string()
            ),
        })
        .is_err());
    }

    #[test]
    fn calendar_pre_mutation_failures_are_retry_safe_only_when_unchanged_is_verified() {
        if crate::tools::system_calendar_event::register_task_tool().is_err() {
            assert!(schema("create_conflict_free_calendar_event").is_ok());
        }
        for code in [
            "calendar_action_denied",
            "calendar_not_found",
            "calendar_name_ambiguous",
            "calendar_read_only",
            "calendar_availability_unsupported",
            "calendar_permission_denied",
            "calendar_permission_restricted",
            "calendar_permission_write_only",
            "calendar_permission_unavailable",
            "calendar_authorization_timeout",
            "calendar_agenda_binding_changed",
        ] {
            let raw = serde_json::json!({
                "taskToolError": {
                    "code": code,
                    "message": "Calendar Full Access is required.",
                    "context": {"changedState": false},
                }
            })
            .to_string();
            let normalized = normalize_agent_error("create_conflict_free_calendar_event", &raw);
            assert!(
                parse_retry_safe_unchanged_error(
                    "create_conflict_free_calendar_event",
                    &normalized
                )
                .is_some(),
                "{code}"
            );

            let unverified = normalize_agent_error(
                "create_conflict_free_calendar_event",
                &serde_json::json!({
                    "taskToolError": {
                        "code": code,
                        "message": "Calendar Full Access is required.",
                        "context": {},
                    }
                })
                .to_string(),
            );
            assert!(
                parse_retry_safe_unchanged_error(
                    "create_conflict_free_calendar_event",
                    &unverified
                )
                .is_none(),
                "{code} must fail closed without verified unchanged state"
            );
        }
    }

    #[test]
    fn mail_failures_are_retry_safe_only_for_allowlisted_verified_unchanged_states() {
        let _ = crate::tools::system_mail::register_task_tool();
        let _ = crate::tools::decision_pack_mail::register_task_tool();
        let _ = crate::tools::release_recovery::register_task_tools();
        for operation in [
            "draft_system_email",
            "draft_decision_pack_email",
            "draft_release_recovery_email",
        ] {
            assert!(schema(operation).is_ok());
            for code in [
                "mail_automation_permission_required",
                "mail_automation_timeout",
                "mail_automation_unavailable",
                "mail_draft_creation_failed_cleanly",
                "mail_agenda_binding_changed",
            ] {
                let normalized = normalize_agent_error(
                    operation,
                    &json!({
                        "taskToolError": {
                            "code": code,
                            "message": "The Mail step stopped before leaving an unverified draft.",
                            "context": {
                                "failurePhase": "preflight",
                                "changedState": false,
                            },
                        }
                    })
                    .to_string(),
                );
                assert!(
                    parse_retry_safe_unchanged_error(operation, &normalized).is_some(),
                    "{operation}/{code}"
                );

                let unverified = normalize_agent_error(
                    operation,
                    &json!({
                        "taskToolError": {
                            "code": code,
                            "message": "The Mail step returned incomplete evidence.",
                            "context": {"failurePhase": "preflight"},
                        }
                    })
                    .to_string(),
                );
                assert!(
                    parse_retry_safe_unchanged_error(operation, &unverified).is_none(),
                    "{operation}/{code} must fail closed without verified unchanged state"
                );
            }

            for (code, changed_state) in [
                ("mail_draft_review_required", json!("external_changes")),
                ("mail_draft_result_unverified", Value::Null),
            ] {
                let mut context = serde_json::Map::new();
                context.insert("failurePhase".to_string(), json!("cleanup"));
                if !changed_state.is_null() {
                    context.insert("changedState".to_string(), changed_state);
                }
                let normalized = normalize_agent_error(
                    operation,
                    &json!({
                        "taskToolError": {
                            "code": code,
                            "message": "Review Mail before continuing.",
                            "context": context,
                        }
                    })
                    .to_string(),
                );
                assert!(
                    parse_retry_safe_unchanged_error(operation, &normalized).is_none(),
                    "{operation}/{code} must never be retry-safe"
                );
            }
        }
    }
}
