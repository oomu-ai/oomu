use crate::network_policy::{DestinationTransport, LocalOriginGrant};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
use std::sync::OnceLock;

pub const SECURITY_TOKEN_REDACTION: &str = "[REDACTED_SECURITY_TOKEN]";
pub const PII_REDACTION: &str = "[REDACTED_PII]";
const REMOTE_STRUCTURAL_REDACTION_MAX_DEPTH: usize = 64;
const REMOTE_STRUCTURAL_REDACTION_MAX_NODES: usize = 50_000;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpTransportConfig {
    Stdio,
    Native,
    Http {
        url: String,
        #[serde(default, rename = "localOriginGrant")]
        local_origin_grant: Option<LocalOriginGrant>,
    },
    Sse {
        url: String,
        #[serde(default, rename = "localOriginGrant")]
        local_origin_grant: Option<LocalOriginGrant>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShieldError {
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteClass {
    Local,
    RemoteHttp,
}

impl Default for McpTransportConfig {
    fn default() -> Self {
        Self::Stdio
    }
}

impl McpTransportConfig {
    pub fn route_class(&self) -> Result<RouteClass, ShieldError> {
        match self {
            Self::Stdio | Self::Native => Ok(RouteClass::Local),
            // Route classification is descriptive only. Native authority is
            // granted later by `network_policy::resolve_destination`, never by
            // renderer-provided transport metadata.
            Self::Http { .. } | Self::Sse { .. } => Ok(RouteClass::RemoteHttp),
        }
    }

    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::Stdio | Self::Native)
    }

    pub fn endpoint(&self) -> Option<&str> {
        match self {
            Self::Stdio | Self::Native => None,
            Self::Http { url, .. } | Self::Sse { url, .. } => Some(url),
        }
    }

    pub fn destination_transport(&self) -> Option<DestinationTransport> {
        match self {
            Self::Stdio | Self::Native => None,
            Self::Http { .. } => Some(DestinationTransport::RemoteMcpHttp),
            Self::Sse { .. } => Some(DestinationTransport::RemoteMcpSse),
        }
    }

    pub fn local_origin_grant(&self) -> Option<LocalOriginGrant> {
        match self {
            Self::Http {
                local_origin_grant, ..
            }
            | Self::Sse {
                local_origin_grant, ..
            } => local_origin_grant.clone(),
            Self::Stdio | Self::Native => None,
        }
    }
}

pub fn sanitize_outgoing_payload_for_transport(
    payload: &str,
    transport: &McpTransportConfig,
) -> Result<String, ShieldError> {
    match transport.route_class()? {
        RouteClass::Local => Ok(payload.to_string()),
        RouteClass::RemoteHttp => {
            // Remote MCP messages are JSON by construction. Apply bounded
            // structural redaction consistent with the approval surface before
            // the exact payload reaches the network boundary; regex-only
            // filtering can miss key variants such as `credentials`,
            // `private_key`, and `session_cookie`.
            let mut value = serde_json::from_str(payload).map_err(|_| ShieldError {
                message: "Remote MCP payload was not valid JSON at the transport boundary."
                    .to_string(),
            })?;
            let mut visited = 0;
            redact_remote_sensitive_fields(&mut value, 0, &mut visited)?;
            let serialized = serde_json::to_string(&value).map_err(|_| ShieldError {
                message: "Remote MCP payload could not be serialized after structural redaction."
                    .to_string(),
            })?;
            Ok(sanitize_payload(&serialized).into_owned())
        }
    }
}

fn redact_remote_sensitive_fields(
    value: &mut serde_json::Value,
    depth: usize,
    visited: &mut usize,
) -> Result<(), ShieldError> {
    *visited = visited.saturating_add(1);
    if depth > REMOTE_STRUCTURAL_REDACTION_MAX_DEPTH
        || *visited > REMOTE_STRUCTURAL_REDACTION_MAX_NODES
    {
        return Err(ShieldError {
            message: "Remote MCP payload exceeded the structural redaction boundary.".to_string(),
        });
    }
    match value {
        serde_json::Value::Object(entries) => {
            for (key, entry) in entries {
                if remote_sensitive_key(key) {
                    *entry = serde_json::Value::String(SECURITY_TOKEN_REDACTION.to_string());
                }
                redact_remote_sensitive_fields(entry, depth + 1, visited)?;
            }
        }
        serde_json::Value::Array(entries) => {
            for entry in entries {
                redact_remote_sensitive_fields(entry, depth + 1, visited)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn remote_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "authorization",
        "privatekey",
        "apikey",
        "password",
        "passwd",
        "credential",
        "secret",
        "token",
        "cookie",
    ]
    .iter()
    .any(|sensitive| normalized.contains(sensitive))
}

pub fn sanitize_payload(payload: &str) -> Cow<'_, str> {
    let mut sanitized = Cow::Borrowed(payload);

    sanitized = replace_if_changed(home_partition_path_regex(), sanitized, "~/");
    sanitized = replace_if_changed(windows_home_path_regex(), sanitized, "~/");
    sanitized = redact_json_secret_values(sanitized);
    sanitized = redact_assignment_secret_values(sanitized);
    sanitized = redact_bearer_tokens(sanitized);
    sanitized = replace_if_changed(security_token_regex(), sanitized, SECURITY_TOKEN_REDACTION);
    sanitized = replace_if_changed(email_regex(), sanitized, PII_REDACTION);
    sanitized = replace_if_changed(phone_regex(), sanitized, PII_REDACTION);
    replace_if_changed(social_identifier_regex(), sanitized, PII_REDACTION)
}

fn replace_if_changed<'a>(
    regex: Option<&Regex>,
    input: Cow<'a, str>,
    replacement: &str,
) -> Cow<'a, str> {
    let Some(regex) = regex else {
        return input;
    };
    if regex.is_match(input.as_ref()) {
        Cow::Owned(regex.replace_all(input.as_ref(), replacement).into_owned())
    } else {
        input
    }
}

fn redact_bearer_tokens(input: Cow<'_, str>) -> Cow<'_, str> {
    let Some(regex) = bearer_token_regex() else {
        return input;
    };
    if regex.is_match(input.as_ref()) {
        Cow::Owned(
            regex
                .replace_all(input.as_ref(), |captures: &regex::Captures<'_>| {
                    format!("{}{}", &captures[1], SECURITY_TOKEN_REDACTION)
                })
                .into_owned(),
        )
    } else {
        input
    }
}

fn redact_json_secret_values(input: Cow<'_, str>) -> Cow<'_, str> {
    let Some(regex) = json_secret_value_regex() else {
        return input;
    };
    if regex.is_match(input.as_ref()) {
        Cow::Owned(
            regex
                .replace_all(input.as_ref(), |captures: &regex::Captures<'_>| {
                    format!(
                        "{}{}{}",
                        &captures[1], SECURITY_TOKEN_REDACTION, &captures[3]
                    )
                })
                .into_owned(),
        )
    } else {
        input
    }
}

fn redact_assignment_secret_values(input: Cow<'_, str>) -> Cow<'_, str> {
    let Some(regex) = assignment_secret_value_regex() else {
        return input;
    };
    if regex.is_match(input.as_ref()) {
        Cow::Owned(
            regex
                .replace_all(input.as_ref(), |captures: &regex::Captures<'_>| {
                    format!("{}{}", &captures[1], SECURITY_TOKEN_REDACTION)
                })
                .into_owned(),
        )
    } else {
        input
    }
}

fn home_partition_path_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"[/]Users/[A-Za-z0-9._-]+/").ok())
        .as_ref()
}

fn windows_home_path_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"(?i)\b[A-Z]:[\\/]+Users[\\/]+[A-Za-z0-9._-]+[\\/]+").ok())
        .as_ref()
}

fn bearer_token_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"(?i)\b(bearer\s+)([A-Za-z0-9._~+/=-]{4,})").ok())
        .as_ref()
}

fn security_token_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)
            \b(
                sk-[A-Za-z0-9_-]{4,}
                |ai-[A-Za-z0-9_-]{8,}
                |ghp_[A-Za-z0-9_]{8,}
                |gho_[A-Za-z0-9_]{8,}
                |github_pat_[A-Za-z0-9_]{8,}
                |xox[baprs]-[A-Za-z0-9-]{8,}
                |[A-Za-z0-9+/]{32,}={0,2}
            )\b
            |(?i)(password|passwd|api[_-]?key|secret|token)(\s*[:=]\s*)[^\s,;}\]]+
            ",
            )
            .ok()
        })
        .as_ref()
}

fn json_secret_value_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r#"(?ix)
            ("(?:
                authorization
                |password
                |passwd
                |api[_-]?key
                |secret
                |access[_-]?token
                |refresh[_-]?token
                |[A-Z0-9_-]*API[_-]?KEY
                |[A-Z0-9_-]*TOKEN
                |[A-Z0-9_-]*SECRET
                |[A-Z0-9_-]*PASSWORD
            )"\s*:\s*")
            ([^"]+)
            (")
            "#,
            )
            .ok()
        })
        .as_ref()
}

fn assignment_secret_value_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r#"(?ix)
            \b((?:
                authorization
                |password
                |passwd
                |api[_-]?key
                |secret
                |access[_-]?token
                |refresh[_-]?token
                |[A-Z0-9_-]*API[_-]?KEY
                |[A-Z0-9_-]*TOKEN
                |[A-Z0-9_-]*SECRET
                |[A-Z0-9_-]*PASSWORD
            )\s*[:=]\s*)
            [^\s,;}\]]+
            "#,
            )
            .ok()
        })
        .as_ref()
}

fn email_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").ok())
        .as_ref()
}

fn phone_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(r"\b(?:\+?1[-.\s]?)?(?:\(?\d{3}\)?[-.\s]?)\d{3}[-.\s]?\d{4}\b").ok()
        })
        .as_ref()
}

fn social_identifier_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").ok())
        .as_ref()
}

impl fmt::Display for ShieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for ShieldError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_security_token_and_home_path_before_remote_transmission() {
        let payload =
            r#"{"auth":"bearer sk-1234abcd","path":"/Users/example/OOMU/sandbox/test.txt"}"#;
        let sanitized = sanitize_payload(payload);

        assert!(sanitized.contains("bearer [REDACTED_SECURITY_TOKEN]"));
        assert!(sanitized.contains("~/OOMU/sandbox/test.txt"));
        assert!(!sanitized.contains("sk-1234abcd"));
        assert!(!sanitized.contains("/Users/example/"));
    }

    #[test]
    fn leaves_local_stdio_payload_unchanged() {
        let payload = r#"{"auth":"bearer sk-1234abcd","path":"/Users/example/SecretProject"}"#;
        let routed =
            sanitize_outgoing_payload_for_transport(payload, &McpTransportConfig::Stdio).unwrap();

        assert_eq!(routed, payload);
    }

    #[test]
    fn sanitizes_json_secret_fields_and_windows_paths_without_breaking_json() {
        let payload = r#"{"api_key":"plain-secret-value","OPENAI_API_KEY":"ai-1234567890abcdef","path":"C:\\Users\\Alex\\OOMU\\sandbox\\test.txt"}"#;
        let sanitized = sanitize_payload(payload);
        let parsed: serde_json::Value =
            serde_json::from_str(&sanitized).expect("sanitized payload remains JSON");

        assert_eq!(
            parsed.get("api_key").and_then(serde_json::Value::as_str),
            Some(SECURITY_TOKEN_REDACTION)
        );
        assert_eq!(
            parsed
                .get("OPENAI_API_KEY")
                .and_then(serde_json::Value::as_str),
            Some(SECURITY_TOKEN_REDACTION)
        );
        assert_eq!(
            parsed.get("path").and_then(serde_json::Value::as_str),
            Some("~/OOMU\\sandbox\\test.txt")
        );
    }

    #[test]
    fn sanitizes_only_remote_http_payloads() {
        let payload = r#"{"phone":"212-555-1212","email":"operator@example.com"}"#;
        let routed = sanitize_outgoing_payload_for_transport(
            payload,
            &McpTransportConfig::Http {
                url: "https://mcp.private-cloud.example/rpc".to_string(),
                local_origin_grant: None,
            },
        )
        .unwrap();

        let routed: serde_json::Value = serde_json::from_str(&routed).unwrap();
        assert_eq!(
            routed,
            serde_json::json!({
                "phone": PII_REDACTION,
                "email": PII_REDACTION,
            })
        );
    }

    #[test]
    fn structurally_redacts_sensitive_key_variants_before_remote_transmission() {
        let payload = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"arguments":{"credentials":"credential-canary","session_cookie":"cookie-canary","nested":{"private-key":"private-key-canary"},"safe":"visible"}},"id":1}"#;
        let routed = sanitize_outgoing_payload_for_transport(
            payload,
            &McpTransportConfig::Http {
                url: "https://mcp.example/rpc".to_string(),
                local_origin_grant: None,
            },
        )
        .unwrap();

        for canary in ["credential-canary", "cookie-canary", "private-key-canary"] {
            assert!(!routed.contains(canary), "leaked {canary}: {routed}");
        }
        let parsed: serde_json::Value = serde_json::from_str(&routed).unwrap();
        assert_eq!(
            parsed
                .pointer("/params/arguments/safe")
                .and_then(serde_json::Value::as_str),
            Some("visible")
        );
    }

    #[test]
    fn route_class_is_descriptive_and_does_not_grant_loopback_authority() {
        let local_sse = McpTransportConfig::Sse {
            url: "http://127.0.0.1:8080/sse".to_string(),
            local_origin_grant: None,
        };
        assert_eq!(local_sse.route_class().unwrap(), RouteClass::RemoteHttp);
        assert!(local_sse.local_origin_grant().is_none());
    }
}
