use super::AgenticLoopError;

pub(super) fn from_operation(operation: &str, message: String) -> AgenticLoopError {
    let (code, boundary) = crate::tools::task_tool_runtime::agent_error_metadata(operation);
    AgenticLoopError {
        code,
        boundary,
        message: crate::tools::task_tool_runtime::normalize_agent_error(operation, &message),
        mlc_path: None,
    }
}

pub(super) fn from_connector(message: String) -> AgenticLoopError {
    AgenticLoopError {
        code: "connector_task_execution_failed",
        boundary: "ConnectorTaskRuntime",
        message,
        mlc_path: None,
    }
}
