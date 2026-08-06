use super::InferenceError;
use reqwest::{StatusCode, Url};

impl InferenceError {
    pub(super) fn project_provider_consent_required() -> Self {
        Self {
            code: "project_provider_consent_required".to_string(),
            boundary: "project_policy_preflight".to_string(),
            message: "This Project asks before using a configured cloud model. Review the exact destination to continue this message."
                .to_string(),
        }
    }

    pub(super) fn project_provider_blocked() -> Self {
        Self {
            code: "project_provider_blocked".to_string(),
            boundary: "project_policy".to_string(),
            message: "This Project keeps its work on this Mac. Choose a local model or change the Project privacy setting."
                .to_string(),
        }
    }

    pub(super) fn project_provider_confirmation_invalid() -> Self {
        Self {
            code: "project_provider_confirmation_invalid".to_string(),
            boundary: "project_policy_preflight".to_string(),
            message: "This cloud approval expired or no longer matches the selected destination. Review the destination again."
                .to_string(),
        }
    }

    pub(super) fn approved_file_unavailable() -> Self {
        Self {
            code: "approved_file_unavailable".to_string(),
            boundary: "approved_file_context".to_string(),
            message: "The approved file context was not available for this response. Please choose the file again."
                .to_string(),
        }
    }

    pub(super) fn chat_turn_already_running() -> Self {
        Self {
            code: "chat_turn_already_running".to_string(),
            boundary: "chat_turn_response_claim".to_string(),
            message: String::from("OOMU is already working on this message. Reply pending."),
        }
    }

    pub(super) fn chat_turn_persistence_failed() -> Self {
        Self {
            code: "chat_turn_persistence_failed".to_string(),
            boundary: "chat_turn_response_claim".to_string(),
            message: "OOMU could not reserve this response. Try again.".to_string(),
        }
    }

    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_request".to_string(),
            boundary: "rust_backend".to_string(),
            message: message.into(),
        }
    }

    pub(super) fn routing_attention(
        code: impl Into<String>,
        boundary: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            boundary: boundary.into(),
            message: message.into(),
        }
    }

    pub(super) fn credential(message: impl Into<String>) -> Self {
        Self {
            code: "credential_unavailable".to_string(),
            boundary: "native_keychain".to_string(),
            message: message.into(),
        }
    }

    pub(super) fn network(message: impl Into<String>) -> Self {
        Self {
            code: "provider_network_error".to_string(),
            boundary: "rust_backend".to_string(),
            message: redact_provider_error_text(&message.into()),
        }
    }

    pub(super) fn network_from_reqwest(error: reqwest::Error) -> Self {
        if error.status() == Some(StatusCode::TOO_MANY_REQUESTS) {
            return Self::provider_rate_limited();
        }
        Self::network(provider_http_error_message(&error))
    }

    pub(super) fn provider_rate_limited() -> Self {
        Self {
            code: "provider_rate_limited".to_string(),
            boundary: "provider_api".to_string(),
            message: "The remote provider rate limit was reached (HTTP 429 Too Many Requests). Wait before retrying, choose another provider or model, or switch to a ready local model."
                .to_string(),
        }
    }

    pub(super) fn provider(message: impl Into<String>) -> Self {
        Self {
            code: "provider_response_error".to_string(),
            boundary: "provider_api".to_string(),
            message: message.into(),
        }
    }

    pub(super) fn deepseek_reasoning_without_answer() -> Self {
        Self {
            code: "deepseek_reasoning_without_answer".to_string(),
            boundary: "provider_api".to_string(),
            message: "DeepSeek completed its hidden reasoning without returning an answer."
                .to_string(),
        }
    }

    pub(super) fn provider_stream_interrupted_after_tokens(message: impl Into<String>) -> Self {
        Self {
            code: "provider_stream_interrupted_after_tokens".to_string(),
            boundary: "provider_api".to_string(),
            message: redact_provider_error_text(&message.into()),
        }
    }

    pub(super) fn worker(message: impl Into<String>) -> Self {
        Self {
            code: "worker_error".to_string(),
            boundary: "rust_backend".to_string(),
            message: message.into(),
        }
    }

    pub(super) fn mod_gated(message: impl Into<String>) -> Self {
        Self {
            code: "mod_requirement_blocked".to_string(),
            boundary: "ModsSubsystem".to_string(),
            message: message.into(),
        }
    }

    pub(super) fn local_infer(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            boundary: "local_infer".to_string(),
            message: message.into(),
        }
    }

    pub(super) fn retry_exhausted(error: &InferenceError, attempts: usize) -> Self {
        Self {
            code: "inference_retry_exhausted".to_string(),
            boundary: error.boundary.clone(),
            message: format!(
                "Transient inference failed after {attempts} attempts. Final error code={}.",
                error.code
            ),
        }
    }
}

pub(super) fn provider_http_error_message(error: &reqwest::Error) -> String {
    if let Some(status) = error.status() {
        return provider_http_status_message(status, error.url());
    }
    let mut message = error.to_string();
    if let Some(url) = error.url() {
        message = message.replace(url.as_str(), &redacted_provider_url(url));
    }
    redact_provider_error_text(&message)
}

pub(super) fn provider_http_status_message(status: StatusCode, url: Option<&Url>) -> String {
    let reason = status.canonical_reason().unwrap_or("HTTP error");
    let mut message = format!(
        "Provider HTTP request failed with status {} {reason}.",
        status.as_u16()
    );
    if let Some(url) = url {
        message.push_str(&format!(" URL: {}", redacted_provider_url(url)));
    }
    message
}

fn redacted_provider_url(url: &Url) -> String {
    let mut redacted = url.clone();
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted.to_string()
}

pub(super) fn redact_provider_error_text(message: &str) -> String {
    let mut redacted = message.to_string();
    for key in [
        "key",
        "api_key",
        "apikey",
        "access_token",
        "token",
        "client_secret",
    ] {
        redacted = redact_query_param(&redacted, key);
    }
    redacted
}

fn redact_query_param(message: &str, key: &str) -> String {
    let mut result = String::with_capacity(message.len());
    let mut remaining = message;
    let needle = format!("{key}=");
    while let Some(offset) = remaining.to_ascii_lowercase().find(&needle) {
        let value_start = offset + needle.len();
        result.push_str(&remaining[..value_start]);
        result.push_str("[redacted]");
        let tail = &remaining[value_start..];
        let value_end = tail
            .find(|character: char| {
                matches!(character, '&' | ')' | ']' | '}' | ' ' | '\n' | '\r' | '\t')
            })
            .unwrap_or(tail.len());
        remaining = &tail[value_end..];
    }
    result.push_str(remaining);
    result
}
