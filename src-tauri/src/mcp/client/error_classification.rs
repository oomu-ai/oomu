use serde_json::Value;

pub(super) fn safe_mcp_tool_error_classification(structured: Option<&Value>) -> Option<Value> {
    let object = structured?.as_object()?;
    let mut safe = serde_json::Map::new();
    for (field, allowed) in [
        (
            "status",
            ["permission_blocked_or_timed_out", "error"].as_slice(),
        ),
        ("warning", ["timeout"].as_slice()),
        ("error_type", ["timeout", "execution_failed"].as_slice()),
    ] {
        let Some(value) = object.get(field).and_then(Value::as_str) else {
            continue;
        };
        if allowed.contains(&value) {
            safe.insert(field.to_string(), Value::String(value.to_string()));
        }
    }
    for field in ["cleanupVerified", "residualDraftPossible"] {
        if let Some(value) = object.get(field).and_then(Value::as_bool) {
            safe.insert(field.to_string(), Value::Bool(value));
        }
    }
    (!safe.is_empty()).then_some(Value::Object(safe))
}
