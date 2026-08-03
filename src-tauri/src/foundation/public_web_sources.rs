use chrono::DateTime;
use serde_json::Value;
use std::collections::HashSet;
use url::Url;

const MAX_CONTEXT_JSON_BYTES: usize = 512_000;
const MAX_SOURCE_URL_CHARS: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedPageSource {
    pub(crate) url: String,
    pub(crate) accessed_at_utc: String,
}

/// Returns only public pages the native retrieval pipeline actually opened.
/// Search-result targets are intentionally excluded because a result can be
/// skipped, fail retrieval, or redirect to a different final URL.
pub(crate) fn from_context_json(context_json: &str) -> Vec<VerifiedPageSource> {
    if context_json.is_empty() || context_json.len() > MAX_CONTEXT_JSON_BYTES {
        return Vec::new();
    }
    let Ok(context) = serde_json::from_str::<Value>(context_json) else {
        return Vec::new();
    };
    let Some(accessed_at_utc) = context
        .get("accessedAtUtc")
        .and_then(Value::as_str)
        .and_then(exact_utc_timestamp)
    else {
        return Vec::new();
    };
    let Some(pages) = context.get("pages").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    pages
        .iter()
        .filter_map(|page| {
            let url = page
                .get("url")
                .and_then(Value::as_str)
                .and_then(public_https_url)?;
            seen.insert(url.clone()).then_some(VerifiedPageSource {
                url,
                accessed_at_utc: accessed_at_utc.clone(),
            })
        })
        .collect()
}

fn exact_utc_timestamp(value: &str) -> Option<String> {
    let value = value.trim();
    if !value.ends_with('Z') || DateTime::parse_from_rfc3339(value).is_err() {
        return None;
    }
    Some(value.to_string())
}

fn public_https_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_SOURCE_URL_CHARS
        || value.chars().any(char::is_control)
    {
        return None;
    }
    let parsed = Url::parse(value).ok()?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opened_final_pages_are_the_only_citable_source_authority() {
        let context = serde_json::json!({
            "accessedAtUtc": "2026-07-24T11:45:10.136Z",
            "results": [
                {"url": "https://nodejs.org/en/download"},
                {"url": "https://snippet-only.example/result"}
            ],
            "pages": [
                {"url": "https://nodejs.org/en"},
                {"url": "https://nodejs.org/en"},
                {"url": "http://insecure.example/"},
                {"url": "file:///private/etc/passwd"}
            ]
        });

        assert_eq!(
            from_context_json(&context.to_string()),
            vec![VerifiedPageSource {
                url: "https://nodejs.org/en".to_string(),
                accessed_at_utc: "2026-07-24T11:45:10.136Z".to_string(),
            }]
        );
    }

    #[test]
    fn malformed_oversized_or_vaguely_timed_context_is_not_authority() {
        assert!(from_context_json("not-json").is_empty());
        assert!(from_context_json(&"x".repeat(MAX_CONTEXT_JSON_BYTES + 1)).is_empty());
        assert!(from_context_json(
            &serde_json::json!({
                "accessedAtUtc": "current turn",
                "pages": [{"url": "https://example.com/"}]
            })
            .to_string()
        )
        .is_empty());
    }
}
