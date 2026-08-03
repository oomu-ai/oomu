use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};
use std::{borrow::Cow, sync::LazyLock};

const REDACTED: &str = "[redacted]";
const MAX_SUMMARY_BYTES: usize = 4096;
const MAX_REDACTION_INPUT_BYTES: usize = 64 * 1024;
const MAX_REDACTION_DEPTH: usize = 12;
const MAX_REDACTION_NODES: usize = 2048;
const LIMIT_MARKER: &str = "[redacted-structure-limit]";
const TRUNCATED_MARKER: &str = "...[truncated]";

static BEARER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=:-]+").expect("valid bearer regex")
});
static SENSITIVE_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(authorization|proxy-authorization|cookie|set-cookie|x-api-key|x-goog-api-key)\s*:[^\r\n]*",
    )
    .expect("valid sensitive header regex")
});
static TELEGRAM_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(/bot)[0-9]{5,}:[A-Za-z0-9_-]{12,}").expect("valid telegram regex")
});
static ASSIGNMENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?ix)(authorization|proxy[-_ ]?authorization|api[-_ ]?key|apikey|password|passwd|client[-_ ]?secret|secret|credentials?|access[-_ ]?token|refresh[-_ ]?token|token|set[-_ ]?cookie|cookie|private[-_ ]?key)\s*([:=])\s*(?:\"[^\"]*\"|'[^']*'|[^\s,;&}\]]+)"#,
    )
    .expect("valid assignment regex")
});
static CLI_SECRET_FLAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?ix)(--?(?:authorization|api[-_]?key|password|passwd|client[-_]?secret|secret|credentials?|access[-_]?token|refresh[-_]?token|token|cookie|private[-_]?key))(\s+|=)(?:\"[^\"]*\"|'[^']*'|[^\s,;&}\]]+)"#,
    )
    .expect("valid CLI secret flag regex")
});
static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s<>"']+"#).expect("valid URL regex"));
static POSIX_HOME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:(?:/Users)|(?:/home))/[^/\s:]+").expect("valid POSIX home path regex")
});
static WINDOWS_HOME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[A-Z]:\\Users\\[^\\\s:]+").expect("valid Windows home path regex")
});

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveField {
    pub path: String,
    pub class: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveDataReport {
    pub contains_sensitive_data: bool,
    pub findings: Vec<SensitiveField>,
}

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn sensitive_class(key: &str) -> Option<&'static str> {
    let key = normalized_key(key);
    if key.contains("authorization") {
        Some("authorization")
    } else if key.contains("privatekey") {
        Some("private_key")
    } else if key.contains("apikey") {
        Some("api_key")
    } else if key.contains("password") || key.contains("passwd") {
        Some("password")
    } else if key.contains("credential") {
        Some("credential")
    } else if key.contains("secret") {
        Some("secret")
    } else if key.contains("token") {
        Some("token")
    } else if key.contains("cookie") {
        Some("cookie")
    } else {
        None
    }
}

fn walk_sensitive(
    value: &Value,
    path: &str,
    depth: usize,
    visited: &mut usize,
    findings: &mut Vec<SensitiveField>,
) {
    *visited = visited.saturating_add(1);
    if depth >= MAX_REDACTION_DEPTH || *visited > MAX_REDACTION_NODES {
        findings.push(SensitiveField {
            path: if path.is_empty() {
                "$".to_string()
            } else {
                path.to_string()
            },
            class: "inspection_limit".to_string(),
        });
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, entry) in object {
                if *visited >= MAX_REDACTION_NODES {
                    findings.push(SensitiveField {
                        path: if path.is_empty() {
                            "$".to_string()
                        } else {
                            path.to_string()
                        },
                        class: "inspection_limit".to_string(),
                    });
                    break;
                }
                let next_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if let Some(class) = sensitive_class(key) {
                    *visited = visited.saturating_add(1);
                    findings.push(SensitiveField {
                        path: next_path,
                        class: class.to_string(),
                    });
                } else {
                    walk_sensitive(entry, &next_path, depth + 1, visited, findings);
                }
            }
        }
        Value::Array(array) => {
            for (index, entry) in array.iter().enumerate() {
                if *visited >= MAX_REDACTION_NODES {
                    findings.push(SensitiveField {
                        path: if path.is_empty() {
                            "$".to_string()
                        } else {
                            path.to_string()
                        },
                        class: "inspection_limit".to_string(),
                    });
                    break;
                }
                walk_sensitive(
                    entry,
                    &format!("{path}[{index}]"),
                    depth + 1,
                    visited,
                    findings,
                );
            }
        }
        _ => {}
    }
}

pub fn inspect_sensitive_json(value: &Value) -> SensitiveDataReport {
    let mut findings = Vec::new();
    let mut visited = 0;
    walk_sensitive(value, "", 0, &mut visited, &mut findings);
    SensitiveDataReport {
        contains_sensitive_data: !findings.is_empty(),
        findings,
    }
}

pub fn redact_json_value(value: &Value) -> Value {
    let mut visited = 0;
    redact_json_value_bounded(value, 0, &mut visited)
}

fn redact_json_value_bounded(value: &Value, depth: usize, visited: &mut usize) -> Value {
    *visited = visited.saturating_add(1);
    if depth >= MAX_REDACTION_DEPTH || *visited > MAX_REDACTION_NODES {
        return Value::String(LIMIT_MARKER.to_string());
    }
    match value {
        Value::Object(object) => {
            let mut redacted = Map::new();
            for (key, entry) in object {
                if *visited >= MAX_REDACTION_NODES {
                    redacted.insert(
                        "__redaction_limit__".to_string(),
                        Value::String(LIMIT_MARKER.to_string()),
                    );
                    break;
                }
                let value = if sensitive_class(key).is_some() {
                    *visited = visited.saturating_add(1);
                    Value::String(REDACTED.to_string())
                } else {
                    redact_json_value_bounded(entry, depth + 1, visited)
                };
                redacted.insert(key.clone(), value);
            }
            Value::Object(redacted)
        }
        Value::Array(array) => {
            let mut redacted = Vec::new();
            for entry in array {
                if *visited >= MAX_REDACTION_NODES {
                    redacted.push(Value::String(LIMIT_MARKER.to_string()));
                    break;
                }
                redacted.push(redact_json_value_bounded(entry, depth + 1, visited));
            }
            Value::Array(redacted)
        }
        Value::String(text) => Value::String(redact_text(text)),
        _ => value.clone(),
    }
}

fn redact_url(raw: &str) -> String {
    let trailing = raw
        .chars()
        .rev()
        .take_while(|character| matches!(character, ')' | ']' | '}' | ',' | '.' | ';'))
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let url_text = raw.strip_suffix(&trailing).unwrap_or(raw);
    let Ok(mut url) = reqwest::Url::parse(url_text) else {
        return TELEGRAM_TOKEN_RE
            .replace_all(raw, format!("$1{REDACTED}"))
            .to_string();
    };
    if !url.username().is_empty() {
        let _ = url.set_username(REDACTED);
    }
    if url.password().is_some() {
        let _ = url.set_password(Some(REDACTED));
    }
    let path = TELEGRAM_TOKEN_RE
        .replace_all(url.path(), format!("$1{REDACTED}"))
        .to_string();
    url.set_path(&path);
    if url.query().is_some() {
        let pairs = url
            .query_pairs()
            .map(|(key, value)| {
                let value = if normalized_key(&key) == "key" || sensitive_class(&key).is_some() {
                    REDACTED.to_string()
                } else {
                    value.into_owned()
                };
                (key.into_owned(), value)
            })
            .collect::<Vec<_>>();
        url.query_pairs_mut().clear().extend_pairs(pairs);
    }
    format!("{}{trailing}", url.as_str())
}

pub fn redact_text(text: &str) -> String {
    let bounded_text = if text.len() > MAX_REDACTION_INPUT_BYTES {
        Cow::Owned(truncate_text_slice(text, MAX_REDACTION_INPUT_BYTES))
    } else {
        Cow::Borrowed(text)
    };
    let text = bounded_text.as_ref();
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return serde_json::to_string(&redact_json_value(&value))
            .unwrap_or_else(|_| REDACTED.to_string());
    }
    let urls_redacted = URL_RE.replace_all(text, |captures: &regex::Captures<'_>| {
        redact_url(
            captures
                .get(0)
                .map(|value| value.as_str())
                .unwrap_or_default(),
        )
    });
    let headers_redacted = SENSITIVE_HEADER_RE.replace_all(&urls_redacted, "$1: [redacted]");
    // Redact credential schemes before generic assignments. Otherwise an
    // `Authorization: Bearer value` header would replace only `Bearer` as the
    // assignment value and leave the actual credential behind.
    let bearer_redacted = BEARER_RE.replace_all(&headers_redacted, "$1 [redacted]");
    let cli_redacted = CLI_SECRET_FLAG_RE.replace_all(&bearer_redacted, "$1$2[redacted]");
    let assignments_redacted = ASSIGNMENT_RE.replace_all(&cli_redacted, "$1$2[redacted]");
    let tokens_redacted = TELEGRAM_TOKEN_RE
        .replace_all(&assignments_redacted, format!("$1{REDACTED}"))
        .to_string();
    let homes_redacted = POSIX_HOME_RE.replace_all(&tokens_redacted, "[home]");
    WINDOWS_HOME_RE
        .replace_all(&homes_redacted, "[home]")
        .to_string()
}

/// Network library errors often embed the full request URL. Remove every URL
/// before the value reaches a logger, status payload, or renderer; callers that
/// need endpoint diagnostics should log a reviewed provider identifier instead.
pub fn redact_network_error(text: &str) -> String {
    let without_urls = URL_RE.replace_all(text, "[redacted-url]");
    redacted_log_text(&without_urls)
}

/// Redact untrusted free text and bound it before interpolation into a log
/// line, diagnostic payload, or other observability surface.
pub fn redacted_log_text(text: &str) -> String {
    truncate_redacted_text(redact_text(text), MAX_SUMMARY_BYTES)
}

fn truncate_redacted_text(redacted: String, max_bytes: usize) -> String {
    if redacted.len() <= max_bytes {
        return redacted;
    }
    truncate_text_slice(&redacted, max_bytes)
}

fn truncate_text_slice(text: &str, max_bytes: usize) -> String {
    let content_budget = max_bytes.saturating_sub(TRUNCATED_MARKER.len());
    let mut end = content_budget.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATED_MARKER}", &text[..end])
}

pub fn redacted_argument_summary(value: &Value) -> String {
    let serialized = serde_json::to_string(&redact_json_value(value))
        .unwrap_or_else(|_| "{\"summary\":\"unavailable\"}".to_string());
    truncate_redacted_text(serialized, MAX_SUMMARY_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recursively_redacts_key_variants_and_reports_paths_only() {
        let value = json!({
            "Authorization": "Bearer canary-one",
            "nested": [{"private-key": "canary-two", "safe": "visible"}],
            "api_key": "canary-three"
        });
        let report = inspect_sensitive_json(&value);
        assert_eq!(report.findings.len(), 3);
        let redacted = redact_json_value(&value).to_string();
        for canary in ["canary-one", "canary-two", "canary-three"] {
            assert!(!redacted.contains(canary));
        }
        assert!(redacted.contains("visible"));
    }

    #[test]
    fn redacts_url_query_headers_and_telegram_path_tokens() {
        let text = "request https://api.telegram.org/bot123456:telegram-secret/getUpdates?token=query-secret\nAuthorization: Bearer bearer-secret\nCookie: session=cookie-secret; other=second-cookie-secret\npassword=pw-secret";
        let redacted = redact_text(text);
        for canary in [
            "telegram-secret",
            "query-secret",
            "bearer-secret",
            "cookie-secret",
            "second-cookie-secret",
            "pw-secret",
        ] {
            assert!(!redacted.contains(canary), "leaked {canary}: {redacted}");
        }
        assert!(redacted.contains("[redacted]"));
    }

    #[test]
    fn redacts_space_separated_cli_secret_flags() {
        let redacted = redact_text(
            "helper --api-key cli-key-canary --token 'cli-token-canary' --password=cli-password-canary --safe visible",
        );
        for canary in ["cli-key-canary", "cli-token-canary", "cli-password-canary"] {
            assert!(!redacted.contains(canary), "leaked {canary}: {redacted}");
        }
        assert!(redacted.contains("--safe visible"));
    }

    #[test]
    fn network_error_redaction_removes_the_entire_url() {
        let redacted = redact_network_error(
            "request failed for https://user:pass@example.test/bot123456:telegram-secret?token=query-secret",
        );
        assert!(!redacted.contains("https://"));
        assert!(!redacted.contains("example.test"));
        assert!(redacted.contains("[redacted-url]"));
    }

    #[test]
    fn redacts_user_home_paths_from_errors_and_stacks() {
        let redacted = redact_text(
            "at /Users/alice/private/file.ts:12 and C:\\Users\\alice\\private\\file.ts:3",
        );
        assert!(!redacted.contains("alice"));
        assert_eq!(redacted.matches("[home]").count(), 2);
    }

    #[test]
    fn log_text_is_redacted_and_utf8_safely_bounded() {
        let canary = "token=bounded-log-canary ";
        let output = redacted_log_text(&format!("{canary}{}", "🙂".repeat(2_000)));
        assert!(output.len() <= MAX_SUMMARY_BYTES);
        assert!(output.ends_with(TRUNCATED_MARKER));
        assert!(!output.contains("bounded-log-canary"));
    }

    #[test]
    fn wide_structures_stop_after_the_global_node_budget() {
        let mut object = Map::new();
        for index in 0..10_000 {
            object.insert(format!("api_key_{index}"), json!(format!("canary-{index}")));
        }
        let value = Value::Object(object);
        let report = inspect_sensitive_json(&value);
        assert!(report.findings.len() <= MAX_REDACTION_NODES + 1);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.class == "inspection_limit"));

        let redacted = redact_json_value(&value);
        let redacted_object = redacted.as_object().unwrap();
        assert!(redacted_object.len() <= MAX_REDACTION_NODES + 1);
        assert!(redacted_object.contains_key("__redaction_limit__"));
        assert!(!serde_json::to_string(redacted_object)
            .unwrap()
            .contains("canary-0"));
    }

    #[test]
    fn deeply_nested_json_stops_at_a_constant_structure_marker() {
        let mut value = Value::String("leaf-canary".to_string());
        for _ in 0..100 {
            value = json!({ "nested": value });
        }
        let redacted = redact_json_value(&value).to_string();
        assert!(redacted.contains(LIMIT_MARKER));
        assert!(!redacted.contains("leaf-canary"));
        let report = inspect_sensitive_json(&value);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.class == "inspection_limit"));
    }
}
