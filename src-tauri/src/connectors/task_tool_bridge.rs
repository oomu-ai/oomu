use super::runtime::{
    execute_agent_task_command, planner_tool_context, resolve_task_tool_dependencies,
    validate_task_tool_request, TaskConnectorToolRequest,
};
use crate::{db::PersistenceEngine, shield_gate::ExecuteCommandResponse};
use serde_json::Value;

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(
        crate::tools::task_tool_runtime::TaskToolRegistration {
            operation: "connected_work",
            validate: validate_registration,
            validate_resolved: validate_registration,
            resolve: resolve_registration,
            execute: execute_registration,
            planner_context: Some(planner_tool_context),
            schema: connected_work_schema,
            metadata: crate::tools::task_tool_runtime::TaskToolMetadata {
                description: "Use one Project-enabled connected account through its governed Task capability.",
                risk_tier: crate::tools::task_tool_runtime::TaskToolRiskTier::Network,
                approval_tier: crate::tools::task_tool_runtime::TaskToolApprovalTier::Background,
                agent_error_code: "connector_task_execution_failed",
                agent_error_boundary: "ConnectorTaskRuntime",
                execution_path: "Connected work ran through the Project-bound connector adapter and recorded Task evidence.",
            },
        },
    )
}

fn connected_work_schema() -> Value {
    serde_json::json!({
        "type":"object",
        "properties":{
            "connector_ref":{"type":"string","minLength":1},
            "capability":{"type":"string","minLength":1},
            "arguments":{"type":"object"}
        },
        "required":["connector_ref","capability","arguments"],
        "additionalProperties":false
    })
}

fn validate_registration(
    arguments: Value,
) -> Result<crate::tools::task_tool_runtime::TaskToolValidation, String> {
    let request = serde_json::from_value::<TaskConnectorToolRequest>(arguments)
        .map_err(|_| "connected_work arguments do not match the registered schema.".to_string())?;
    let request = validate_task_tool_request(request)?;
    let potentially_effectful = request.potentially_effectful();
    Ok(crate::tools::task_tool_runtime::TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful,
    })
}

fn resolve_registration(
    _persistence: &PersistenceEngine,
    _execution_id: Option<&str>,
    arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let request = serde_json::from_value::<TaskConnectorToolRequest>(arguments)
        .map_err(|_| "connected_work arguments do not match the registered schema.".to_string())?;
    serde_json::to_value(resolve_task_tool_dependencies(request, outputs)?)
        .map_err(|error| error.to_string())
}

fn execute_registration<'a>(
    context: crate::tools::task_tool_runtime::TaskToolExecutionContext<'a>,
    arguments: Value,
) -> crate::tools::task_tool_runtime::TaskToolFuture<'a> {
    Box::pin(async move {
        let request =
            serde_json::from_value::<TaskConnectorToolRequest>(arguments).map_err(|_| {
                "connected_work arguments do not match the registered schema.".to_string()
            })?;
        execute_agent_task_command(
            context.persistence,
            context.app,
            context.execution_id,
            request,
        )
        .await
    })
}
