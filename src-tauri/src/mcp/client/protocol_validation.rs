use super::{
    error_classification, validate_json_structure, JsonRpcRequest, McpClientError, McpTool,
    MCP_MAX_TOOL_CATALOG_SIZE, MCP_MAX_TOOL_DESCRIPTION_BYTES, MCP_MAX_TOOL_NAME_BYTES,
    MCP_MAX_TOOL_SCHEMA_BYTES,
};
use crate::mcp_result::McpToolCallResult;
use serde_json::Value;
use std::collections::HashSet;

pub(super) fn validate_json_rpc_request(request: &JsonRpcRequest) -> Result<(), McpClientError> {
    if request.jsonrpc != "2.0" {
        return Err(McpClientError::protocol(
            "JSON-RPC request must declare jsonrpc \"2.0\".".to_string(),
        ));
    }
    validate_json_rpc_method(&request.method)?;
    request_id_key(&request.id)?;
    Ok(())
}

pub(super) fn validate_json_rpc_method(method: &str) -> Result<(), McpClientError> {
    if method.trim().is_empty() {
        return Err(McpClientError::protocol(
            "JSON-RPC method must be non-empty.".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn request_id_key(id: &Value) -> Result<String, McpClientError> {
    match id {
        Value::String(_) | Value::Number(_) => serde_json::to_string(id).map_err(|error| {
            McpClientError::protocol(format!("Failed to normalize JSON-RPC id: {error}"))
        }),
        _ => Err(McpClientError::protocol(
            "JSON-RPC request id must be a string or number.".to_string(),
        )),
    }
}

pub(super) fn parse_tools_list(result: Value) -> Result<Vec<McpTool>, McpClientError> {
    let tools: Vec<McpTool> = if let Some(tools) = result.get("tools") {
        serde_json::from_value(tools.clone())
            .map_err(|error| McpClientError::protocol(format!("Invalid MCP tools list: {error}")))
    } else if result.is_array() {
        serde_json::from_value(result)
            .map_err(|error| McpClientError::protocol(format!("Invalid MCP tools list: {error}")))
    } else {
        return Err(McpClientError::protocol(
            "MCP tools/list result must contain a tools array.".to_string(),
        ));
    }?;
    validate_catalog(&tools)?;
    Ok(tools)
}

fn validate_catalog(tools: &[McpTool]) -> Result<(), McpClientError> {
    if tools.len() > MCP_MAX_TOOL_CATALOG_SIZE {
        return Err(McpClientError::protocol(format!(
            "MCP tool catalog exceeded the maximum of {MCP_MAX_TOOL_CATALOG_SIZE} tools."
        )));
    }
    let mut names = HashSet::new();
    for tool in tools {
        validate_tool(tool, &mut names)?;
    }
    Ok(())
}

fn validate_tool<'a>(
    tool: &'a McpTool,
    names: &mut HashSet<&'a str>,
) -> Result<(), McpClientError> {
    if tool.name.trim().is_empty() || tool.name.len() > MCP_MAX_TOOL_NAME_BYTES {
        return Err(McpClientError::protocol(format!(
            "MCP tool name was empty or exceeded the {MCP_MAX_TOOL_NAME_BYTES} byte limit."
        )));
    }
    if !names.insert(tool.name.as_str()) {
        return Err(McpClientError::protocol(
            "MCP tool catalog contained duplicate tool names.".to_string(),
        ));
    }
    if tool.description.len() > MCP_MAX_TOOL_DESCRIPTION_BYTES {
        return Err(McpClientError::protocol(format!(
            "MCP tool description exceeded the {MCP_MAX_TOOL_DESCRIPTION_BYTES} byte limit."
        )));
    }
    validate_tool_schema_size(&tool.input_schema, "input")?;
    if let Some(output_schema) = tool.output_schema.as_ref() {
        validate_tool_schema_size(output_schema, "output")?;
    }
    Ok(())
}

fn validate_tool_schema_size(schema: &Value, label: &str) -> Result<(), McpClientError> {
    validate_json_structure(schema)?;
    let byte_count = serde_json::to_vec(schema)
        .map_err(|error| {
            McpClientError::protocol(format!("MCP {label} schema could not be measured: {error}"))
        })?
        .len();
    if byte_count > MCP_MAX_TOOL_SCHEMA_BYTES {
        return Err(McpClientError::protocol(format!(
            "MCP {label} schema exceeded the {MCP_MAX_TOOL_SCHEMA_BYTES} byte limit."
        )));
    }
    Ok(())
}

pub(super) fn parse_tool_call_result(result: Value) -> Result<McpToolCallResult, McpClientError> {
    let mut parsed = serde_json::from_value::<McpToolCallResult>(result.clone())
        .map_err(|error| McpClientError::protocol(format!("Invalid MCP tool result: {error}")))?;
    if parsed.is_error {
        redact_tool_error(&result, &mut parsed);
        return Ok(parsed);
    }
    if parsed.content.is_empty() && parsed.structured_content.is_none() {
        return Err(McpClientError::protocol(
            "MCP tool returned success without content or structured evidence.".to_string(),
        ));
    }
    Ok(parsed)
}

fn redact_tool_error(result: &Value, parsed: &mut McpToolCallResult) {
    let redacted_summary = crate::redaction::redacted_argument_summary(result);
    let safe_classification = error_classification::safe_mcp_tool_error_classification(
        parsed.structured_content.as_ref(),
    );
    let safe_text = if safe_classification.is_some() {
        "MCP tool returned a typed error.".to_string()
    } else {
        redacted_summary
    };
    parsed.content = vec![serde_json::json!({
        "type": "text",
        "text": safe_text,
    })];
    parsed.structured_content = safe_classification;
    parsed.meta = None;
    parsed.raw = None;
}
