use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const MAX_CODE_CHARS: usize = 128;
const MAX_MESSAGE_CHARS: usize = 1_000;
const MAX_CONTEXT_KEYS: usize = 32;
const MAX_CONTEXT_DEPTH: usize = 4;
const MAX_CONTEXT_STRING_CHARS: usize = 500;
const MAX_CONTEXT_ARRAY_ITEMS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChangedState {
    None,
    CheckpointSaved,
    ExternalChanges,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskToolAgentError {
    pub code: String,
    pub boundary: String,
    pub message: String,
    pub context: Map<String, Value>,
    pub changed_state: ChangedState,
    pub changed_state_verified: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskToolErrorEnvelope {
    task_tool_error: RawTaskToolError,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTaskToolError {
    code: String,
    message: String,
    #[serde(default)]
    context: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NormalizedTaskToolErrorEnvelope {
    task_tool_error: NormalizedTaskToolError,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NormalizedTaskToolError {
    code: String,
    boundary: String,
    message: String,
    context: Map<String, Value>,
    changed_state: String,
    changed_state_verified: bool,
}

pub(crate) fn decode(
    raw: &str,
    fallback_code: &str,
    fallback_boundary: &str,
    fallback_message: &str,
) -> TaskToolAgentError {
    let parsed = serde_json::from_str::<TaskToolErrorEnvelope>(raw).ok();
    let Some(parsed) = parsed else {
        return fallback(fallback_code, fallback_boundary, fallback_message);
    };
    let raw_error = parsed.task_tool_error;
    if !valid_code(&raw_error.code) || !valid_plain_message(&raw_error.message) {
        return fallback(fallback_code, fallback_boundary, fallback_message);
    }
    let mut context = sanitize_context(raw_error.context);
    let (changed_state, changed_state_verified) = take_changed_state(&mut context);
    TaskToolAgentError {
        code: raw_error.code,
        boundary: fallback_boundary.to_string(),
        message: raw_error.message.trim().to_string(),
        context,
        changed_state,
        changed_state_verified,
    }
}

pub(crate) fn encode(error: &TaskToolAgentError) -> String {
    serde_json::json!({
        "taskToolError": {
            "code": error.code,
            "boundary": error.boundary,
            "message": error.message,
            "context": error.context,
            "changedState": error.changed_state,
            "changedStateVerified": error.changed_state_verified,
        }
    })
    .to_string()
}

pub(crate) fn decode_normalized(raw: &str) -> Option<TaskToolAgentError> {
    let envelope = serde_json::from_str::<NormalizedTaskToolErrorEnvelope>(raw).ok()?;
    let error = envelope.task_tool_error;
    if !valid_code(&error.code)
        || !valid_boundary(&error.boundary)
        || !valid_plain_message(&error.message)
    {
        return None;
    }
    let changed_state = match error.changed_state.as_str() {
        "none" => ChangedState::None,
        "checkpoint_saved" => ChangedState::CheckpointSaved,
        "external_changes" => ChangedState::ExternalChanges,
        _ => return None,
    };
    if !error.changed_state_verified && changed_state != ChangedState::None {
        return None;
    }
    Some(TaskToolAgentError {
        code: error.code,
        boundary: error.boundary,
        message: error.message.trim().to_string(),
        context: sanitize_context(error.context),
        changed_state,
        changed_state_verified: error.changed_state_verified,
    })
}

fn fallback(code: &str, boundary: &str, message: &str) -> TaskToolAgentError {
    TaskToolAgentError {
        code: code.to_string(),
        boundary: boundary.to_string(),
        message: message.to_string(),
        context: Map::new(),
        changed_state: ChangedState::None,
        changed_state_verified: false,
    }
}

fn valid_code(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.chars().count() <= MAX_CODE_CHARS
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_boundary(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.chars().count() <= MAX_CODE_CHARS
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_plain_message(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.chars().count() <= MAX_MESSAGE_CHARS
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        && !value.contains('<')
        && !value.contains('>')
}

fn take_changed_state(context: &mut Map<String, Value>) -> (ChangedState, bool) {
    let value = context.remove("changedState");
    match value {
        Some(Value::Bool(false)) => (ChangedState::None, true),
        Some(Value::Bool(true)) => (ChangedState::ExternalChanges, true),
        Some(Value::String(value)) if value == "checkpoint_saved" => {
            (ChangedState::CheckpointSaved, true)
        }
        Some(Value::String(value)) if value == "external_changes" => {
            (ChangedState::ExternalChanges, true)
        }
        _ => (ChangedState::None, false),
    }
}

fn sanitize_context(context: Map<String, Value>) -> Map<String, Value> {
    context
        .into_iter()
        .filter(|(key, _)| valid_context_key(key))
        .take(MAX_CONTEXT_KEYS)
        .filter_map(|(key, value)| sanitize_value(value, 0).map(|value| (key, value)))
        .collect()
}

fn valid_context_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn sanitize_value(value: Value, depth: usize) -> Option<Value> {
    if depth >= MAX_CONTEXT_DEPTH {
        return None;
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value),
        Value::String(value) => Some(Value::String(
            value.chars().take(MAX_CONTEXT_STRING_CHARS).collect(),
        )),
        Value::Array(values) => Some(Value::Array(
            values
                .into_iter()
                .take(MAX_CONTEXT_ARRAY_ITEMS)
                .filter_map(|value| sanitize_value(value, depth + 1))
                .collect(),
        )),
        Value::Object(values) => Some(Value::Object(
            values
                .into_iter()
                .filter(|(key, _)| valid_context_key(key))
                .take(MAX_CONTEXT_KEYS)
                .filter_map(|(key, value)| {
                    sanitize_value(value, depth + 1).map(|value| (key, value))
                })
                .collect(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_safe_typed_error_and_extracts_changed_state() {
        let decoded = decode(
            r#"{"taskToolError":{"code":"decision_pack_creation_failed","message":"Recent official freight evidence was not verified.","context":{"kind":"research_qualification","subject":"freight","verifiedInputs":2,"changedState":"checkpoint_saved"}}}"#,
            "registered_task_tool_failed",
            "DecisionPack",
            "The task could not finish safely.",
        );
        assert_eq!(decoded.code, "decision_pack_creation_failed");
        assert_eq!(decoded.boundary, "DecisionPack");
        assert_eq!(decoded.context["subject"], "freight");
        assert!(!decoded.context.contains_key("changedState"));
        assert_eq!(decoded.changed_state, ChangedState::CheckpointSaved);
        assert!(decoded.changed_state_verified);
        assert_eq!(decode_normalized(&encode(&decoded)), Some(decoded));
    }

    #[test]
    fn malformed_or_markup_error_falls_back_without_retaining_raw_content() {
        for raw in [
            "ordinary internal error",
            r#"{"taskToolError":{"code":"BAD CODE","message":"unsafe","context":{}}}"#,
            r#"{"taskToolError":{"code":"safe_code","message":"<script>bad</script>","context":{}}}"#,
        ] {
            let decoded = decode(
                raw,
                "fixture_failed",
                "Fixture",
                "The fixture could not finish safely.",
            );
            assert_eq!(decoded.code, "fixture_failed");
            assert_eq!(decoded.message, "The fixture could not finish safely.");
            assert!(decoded.context.is_empty());
            assert!(!decoded.changed_state_verified);
        }
    }

    #[test]
    fn omitted_or_invalid_change_state_never_becomes_verified_unchanged() {
        for raw in [
            r#"{"taskToolError":{"code":"safe_code","message":"Safe failure.","context":{}}}"#,
            r#"{"taskToolError":{"code":"safe_code","message":"Safe failure.","context":{"changedState":"none"}}}"#,
        ] {
            let decoded = decode(raw, "fallback", "Fixture", "Safe fallback.");
            assert_eq!(decoded.changed_state, ChangedState::None);
            assert!(!decoded.changed_state_verified);
        }
    }
}
