use super::*;

pub(super) fn validate_generated_tool_schema(value: &Value) -> Result<(), GemmaError> {
    let invalid = |message: String| GemmaError {
        code: "gemma_action_plan_schema_invalid",
        message,
    };
    let tool = value
        .as_object()
        .ok_or_else(|| invalid("ActionPlan tool must be an object.".to_string()))?;
    let kind = tool
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("ActionPlan tool.kind is required.".to_string()))?;
    if kind
        .trim()
        .replace('-', "_")
        .eq_ignore_ascii_case("create_spreadsheet")
    {
        let mut arguments = tool.clone();
        arguments.remove("kind");
        crate::tools::spreadsheet_schema::validate_public_create_spreadsheet_envelope(
            &Value::Object(arguments),
        )
        .map_err(invalid)?;
    }
    if let Some(validation) = crate::tools::task_tool_runtime::validate_generated_tool(kind, tool) {
        validation.map_err(invalid)?;
        return Ok(());
    }
    let registry = crate::tools::registry::NativeToolRegistry::default();
    if registry.contains(kind) {
        let mut arguments = tool.clone();
        arguments.remove("kind");
        registry
            .validate_call(kind, &Value::Object(arguments))
            .map_err(invalid)?;
        return Ok(());
    }
    let required = crate::tools::registry::local_gemma_action_tool_required_fields(kind)
        .ok_or_else(|| {
            invalid(format!(
                "ActionPlan tool kind '{kind}' is not authorized by the schema."
            ))
        })?;
    for field in required {
        let valid = match tool.get(*field) {
            Some(Value::Object(_)) if *field == "arguments" => true,
            Some(Value::String(value)) => {
                matches!(*field, "content" | "replacement_content") || !value.trim().is_empty()
            }
            Some(Value::Array(values)) => {
                !values.is_empty()
                    && values
                        .iter()
                        .all(|value| value.as_str().is_some_and(|item| !item.trim().is_empty()))
            }
            _ => false,
        };
        if !valid {
            return Err(invalid(format!(
                "ActionPlan tool '{kind}' requires valid field '{field}'."
            )));
        }
    }
    Ok(())
}

pub(super) fn parse_terminal_generated_tool(value: &Value) -> Option<GeneratedToolDraft> {
    let mut arguments = value.as_object()?.clone();
    arguments.remove("kind");
    let request = serde_json::from_value::<crate::tools::terminal_contract::NativeTerminalRequest>(
        Value::Object(arguments),
    )
    .ok()?
    .validate()
    .ok()?;
    Some(GeneratedToolDraft::TerminalExecute {
        executable: request.executable,
        args: request.args,
        env: request.env,
        cwd: request.cwd,
        timeout: request.timeout,
    })
}
