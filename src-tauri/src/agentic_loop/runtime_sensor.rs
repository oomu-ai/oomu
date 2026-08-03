pub(super) fn supported_operation(operation: &str) -> bool {
    matches!(
        operation.trim(),
        "codebase_compile" | "terminal_execute" | "shell_command" | "codebase_patch" | "file_write"
    )
}

pub(super) fn mission_id(plan_id: &str, session_id: Option<&str>) -> String {
    session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(plan_id)
        .to_string()
}
