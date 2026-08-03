use crate::{
    db::PersistenceEngine,
    mcp::client::{McpClientError, McpClientRegistry, McpToolApproval, McpToolApprovalBinding},
    mcp_result::McpToolCallResult,
};
use serde_json::Value;
use std::time::Duration;

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_blocking(
    registry: McpClientRegistry,
    persistence: PersistenceEngine,
    execution_id: &str,
    node_id: &str,
    label: &str,
    server_name: &str,
    tool_name: &str,
    arguments: Value,
    timeout_ms: u64,
    approval_binding: Option<McpToolApprovalBinding>,
    human_approved: bool,
) -> Result<McpToolCallResult, super::WorkflowRuntimeError> {
    let error_server_name = server_name.to_string();
    let execution_node_id = node_id.to_string();
    let call = async move {
        tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            execute(
                registry,
                persistence,
                execution_id.to_string(),
                execution_node_id,
                server_name.to_string(),
                tool_name.to_string(),
                arguments,
                approval_binding,
                human_approved,
            ),
        )
        .await
    };
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(call),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| super::WorkflowRuntimeError::runtime(error.to_string()))?
            .block_on(call),
    }
    .map_err(|_| super::WorkflowRuntimeError::node_timeout(node_id, label, timeout_ms))?;
    result.map_err(|error| super::workflow_error_from_mcp_client(&error_server_name, error))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    registry: McpClientRegistry,
    persistence: PersistenceEngine,
    execution_id: String,
    node_id: String,
    server_name: String,
    tool_name: String,
    arguments: Value,
    approval_binding: Option<McpToolApprovalBinding>,
    human_approved: bool,
) -> Result<McpToolCallResult, McpClientError> {
    let approval = if human_approved {
        match approval_binding.as_ref() {
            Some(expected_binding) => registry
                .activate_tool_approval_after_verified_workflow_review(
                    &server_name,
                    &tool_name,
                    arguments.clone(),
                    expected_binding,
                )
                .await?
                .map(|request| McpToolApproval {
                    approval_token: request.approval_token,
                }),
            None => {
                approval_without_binding(&registry, &server_name, &tool_name, &arguments).await?
            }
        }
    } else {
        None
    };
    crate::mcp::client::native_apple_receipts::execute_workflow_tool(
        &registry,
        &persistence,
        &execution_id,
        &node_id,
        &server_name,
        &tool_name,
        arguments,
        approval,
        human_approved,
    )
    .await
    .map_err(McpClientError::protocol)
}

async fn approval_without_binding(
    registry: &McpClientRegistry,
    server_name: &str,
    tool_name: &str,
    arguments: &Value,
) -> Result<Option<McpToolApproval>, McpClientError> {
    #[cfg(test)]
    {
        // The workflow test guard suppresses the UI pause before the blocking worker starts.
        // Still consume an exact, one-use local MCP token at the real boundary.
        registry
            .prepare_tool_approval(server_name, tool_name, arguments.clone())
            .await
            .map(|request| {
                request.map(|request| McpToolApproval {
                    approval_token: request.approval_token,
                })
            })
    }
    #[cfg(not(test))]
    {
        let _ = (registry, server_name, tool_name, arguments);
        Err(McpClientError::permission(
            "The reviewed service details are unavailable. No action was taken.".to_string(),
        ))
    }
}
