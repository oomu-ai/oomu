use super::{ReviewedAction, ReviewedNodeAuthority};
use crate::tools::task_tool_runtime::TASK_RUN_TIMESTAMP_TOKEN;
use chrono::NaiveDateTime;
use serde_json::Value;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub(super) fn verify_path_authority(
    node: &ReviewedNodeAuthority,
    arguments: &Value,
    roots: &[String],
) -> Result<bool, String> {
    let Some(path_pointer) = node.path_pointer.as_deref() else {
        return Ok(false);
    };
    let Some(raw_path) = arguments.pointer(path_pointer).and_then(Value::as_str) else {
        return Ok(false);
    };
    if !node.allowed_extensions.is_empty() {
        let extension = Path::new(raw_path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !node
            .allowed_extensions
            .iter()
            .any(|allowed| allowed == &extension)
        {
            return Ok(false);
        }
    }
    let require_existing = !matches!(node.action, ReviewedAction::VerifiedDocumentWrite);
    for root in roots {
        let root = Path::new(root);
        let candidate = if Path::new(raw_path).is_absolute() {
            PathBuf::from(raw_path)
        } else {
            root.join(raw_path)
        };
        let Some(resolved) = resolve_candidate(&candidate, require_existing)? else {
            continue;
        };
        if !resolved.starts_with(root) {
            continue;
        }
        let valid_kind = match node.action {
            ReviewedAction::ProjectFileRead => resolved.is_file(),
            ReviewedAction::ProjectDirectoryRead => resolved.is_dir(),
            ReviewedAction::VerifiedDocumentWrite => !resolved.exists() || resolved.is_file(),
            ReviewedAction::OfficialPageRead
            | ReviewedAction::DeterministicAnalysis
            | ReviewedAction::NativePersonalDataRead => false,
        };
        if valid_kind {
            return Ok(true);
        }
    }
    Ok(false)
}

fn resolve_candidate(path: &Path, require_existing: bool) -> Result<Option<PathBuf>, String> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Ok(None);
    }
    if path.exists() {
        return fs::canonicalize(path)
            .map(Some)
            .map_err(|error| error.to_string());
    }
    if require_existing {
        return Ok(None);
    }
    let mut missing = Vec::new();
    let mut ancestor = path;
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            return Ok(None);
        };
        missing.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            return Ok(None);
        };
        ancestor = parent;
    }
    let mut resolved = fs::canonicalize(ancestor).map_err(|error| error.to_string())?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(Some(resolved))
}

pub(super) fn arguments_template_matches(
    node: &ReviewedNodeAuthority,
    actual: &Value,
    roots: &[String],
) -> bool {
    let Some(pointer) = node.path_pointer.as_deref() else {
        return template_matches(&node.arguments_template, actual);
    };
    let Some(expected_path) = node
        .arguments_template
        .pointer(pointer)
        .and_then(Value::as_str)
    else {
        return false;
    };
    let Some(actual_path) = actual.pointer(pointer).and_then(Value::as_str) else {
        return false;
    };
    if !path_template_matches(expected_path, actual_path, roots) {
        return false;
    }
    let mut expected = node.arguments_template.clone();
    let mut observed = actual.clone();
    let Some(expected_slot) = expected.pointer_mut(pointer) else {
        return false;
    };
    *expected_slot = Value::Null;
    let Some(observed_slot) = observed.pointer_mut(pointer) else {
        return false;
    };
    *observed_slot = Value::Null;
    template_matches(&expected, &observed)
}

fn template_matches(template: &Value, actual: &Value) -> bool {
    match (template, actual) {
        (Value::Object(expected), Value::Object(observed)) => {
            expected.len() == observed.len()
                && expected.iter().all(|(key, value)| {
                    observed
                        .get(key)
                        .is_some_and(|actual| template_matches(value, actual))
                })
        }
        (Value::Array(expected), Value::Array(observed)) => {
            expected.len() == observed.len()
                && expected
                    .iter()
                    .zip(observed)
                    .all(|(left, right)| template_matches(left, right))
        }
        (Value::String(expected), actual) if exact_template(expected) => !actual.is_null(),
        (Value::String(expected), Value::String(observed)) if expected.contains("{{") => {
            template_string_matches(expected, observed)
        }
        _ => template == actual,
    }
}

fn path_template_matches(template: &str, actual: &str, roots: &[String]) -> bool {
    if exact_template(template) {
        return true;
    }
    let mut candidates = vec![template.to_string()];
    if !Path::new(template).is_absolute() && Path::new(actual).is_absolute() {
        candidates = roots
            .iter()
            .map(|root| Path::new(root).join(template).to_string_lossy().to_string())
            .collect();
    }
    candidates.into_iter().any(|candidate| {
        if candidate == actual {
            true
        } else if candidate.contains(TASK_RUN_TIMESTAMP_TOKEN) {
            timestamp_template_matches(&candidate, actual)
        } else if candidate.contains("{{") {
            template_string_matches(&candidate, actual)
        } else {
            false
        }
    })
}

fn timestamp_template_matches(template: &str, actual: &str) -> bool {
    let mut pieces = template.split(TASK_RUN_TIMESTAMP_TOKEN);
    let Some(prefix) = pieces.next() else {
        return false;
    };
    let Some(suffix) = pieces.next() else {
        return false;
    };
    if pieces.next().is_some() {
        return false;
    }
    let Some(remainder) = actual.strip_prefix(prefix) else {
        return false;
    };
    let Some(timestamp) = remainder.strip_suffix(suffix) else {
        return false;
    };
    timestamp.len() == 16
        && timestamp.is_ascii()
        && NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d_%H-%M").is_ok()
}

fn exact_template(value: &str) -> bool {
    value.starts_with("{{")
        && value.ends_with("}}")
        && value[2..value.len() - 2].find("{{").is_none()
}

fn template_string_matches(template: &str, actual: &str) -> bool {
    let mut cursor = 0usize;
    let mut remainder = template;
    while let Some(start) = remainder.find("{{") {
        let static_prefix = &remainder[..start];
        if !actual[cursor..].starts_with(static_prefix) {
            return false;
        }
        cursor += static_prefix.len();
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find("}}") else {
            return false;
        };
        remainder = &after_start[end + 2..];
        if remainder.is_empty() {
            return true;
        }
        let next_static_end = remainder.find("{{").unwrap_or(remainder.len());
        let next_static = &remainder[..next_static_end];
        let Some(found) = actual[cursor..].find(next_static) else {
            return false;
        };
        cursor += found;
    }
    actual[cursor..] == *remainder
}
