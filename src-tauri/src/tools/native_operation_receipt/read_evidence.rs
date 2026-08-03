use serde_json::Value;

const COLLECTION_KEYS: [&str; 9] = [
    "events",
    "reminders",
    "emails",
    "notes",
    "contacts",
    "photos",
    "songs",
    "uiText",
    "files",
];

pub(super) fn from_structured(value: Option<&Value>) -> (&'static str, Option<u64>) {
    collection(value)
        .or_else(|| scalar_local_file_read(value))
        .unwrap_or(("native_result", None))
}

fn collection(value: Option<&Value>) -> Option<(&'static str, Option<u64>)> {
    let value = value?;
    for key in COLLECTION_KEYS {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            return Some(("bounded_native_collection", Some(items.len() as u64)));
        }
    }
    value
        .get("returnedCount")
        .and_then(Value::as_u64)
        .map(|count| ("bounded_native_collection", Some(count)))
}

fn scalar_local_file_read(value: Option<&Value>) -> Option<(&'static str, Option<u64>)> {
    let object = value?.as_object()?;
    if COLLECTION_KEYS.iter().any(|key| object.contains_key(*key))
        || object.contains_key("returnedCount")
    {
        return None;
    }
    let bound_path = |key| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
    };
    let path = bound_path("path")?;
    let relative_path = bound_path("relativePath")?;
    if path != relative_path {
        return None;
    }
    let content = object.get("content").and_then(Value::as_str)?;
    Some(("bounded_local_file_read", Some(content.len() as u64)))
}

#[cfg(test)]
mod tests {
    use super::super::{evidence_from_mcp_result, NativeActionClass};
    use crate::mcp_result::McpToolCallResult;
    use serde_json::{json, Value};

    fn evidence(structured_content: Value) -> super::super::NativePostconditionEvidence {
        evidence_from_mcp_result(
            NativeActionClass::Read,
            &McpToolCallResult {
                content: Vec::new(),
                structured_content: Some(structured_content),
                is_error: false,
                meta: None,
                raw: None,
            },
        )
    }

    #[test]
    fn verifies_nonempty_and_empty_scalar_file_content() {
        for (content, expected_bytes) in [("verified facts", 14), ("", 0)] {
            let evidence = evidence(json!({
                "path": "reports/input.txt",
                "relativePath": "reports/input.txt",
                "content": content,
            }));
            assert!(evidence.verified);
            assert_eq!(evidence.evidence_kind, "bounded_local_file_read");
            assert_eq!(evidence.bounded_count, Some(expected_bytes));
        }
    }

    #[test]
    fn rejects_malformed_or_collection_shaped_scalar_evidence() {
        for malformed in [
            json!({"path": "input.txt", "content": "facts"}),
            json!({"path": "input.txt", "relativePath": "other.txt", "content": "facts"}),
            json!({"path": " ", "relativePath": " ", "content": "facts"}),
            json!({"path": "input.txt", "relativePath": "input.txt", "content": 4}),
            json!({
                "emails": "not-a-collection",
                "path": "mail.txt",
                "relativePath": "mail.txt",
                "content": "private mail",
            }),
        ] {
            let evidence = evidence(malformed);
            assert!(!evidence.verified);
            assert_eq!(evidence.evidence_kind, "native_result");
            assert_eq!(evidence.bounded_count, None);
        }
    }
}
