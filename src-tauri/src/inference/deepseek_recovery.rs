//! Bounded recovery for provider responses that contain no visible answer.

use super::{
    execute_provider_inference, execute_provider_streaming_inference,
    execute_with_transient_inference_retry, retry_allowed_for_stream,
    validate_inference_request_attachments, ChatEventStream, InferenceError, InferenceRequest,
    InferenceResponse, ResolvedProviderRoute,
};

struct EmptyAnswerRecovery {
    can_reduce_reasoning: bool,
    retry_without_reasoning: bool,
}

impl EmptyAnswerRecovery {
    fn new(request: &InferenceRequest) -> Self {
        let provider = request.provider_id.trim().to_ascii_lowercase();
        let can_reduce_reasoning = matches!(
            provider.as_str(),
            "deepseek"
                | "deepseek_v3"
                | "deepseek_r1"
                | "google"
                | "gemini"
                | "google_gemini"
                | "gemini_pro"
                | "gemini_flash"
        ) && request.reasoning.as_deref().map(str::trim).is_some_and(
            |reasoning| {
                !reasoning.is_empty()
                    && !matches!(
                        reasoning.to_ascii_lowercase().as_str(),
                        "off" | "none" | "disabled" | "false" | "0"
                    )
            },
        );
        Self {
            can_reduce_reasoning,
            retry_without_reasoning: false,
        }
    }

    fn request(&self, request: &InferenceRequest) -> InferenceRequest {
        let mut attempt = request.clone();
        if self.retry_without_reasoning {
            attempt.reasoning = Some("off".to_string());
            attempt.reasoning_budget_tokens = None;
        }
        attempt
    }

    fn error(&mut self, error: InferenceError, model_id: &str) -> InferenceError {
        if !provider_returned_no_visible_answer(&error) {
            return error;
        }
        if self.retry_without_reasoning {
            return InferenceError {
                code: "provider_empty_after_recovery".to_string(),
                boundary: "provider_api".to_string(),
                message: "The provider still returned no visible answer after OOMU reduced reasoning for one retry."
                    .to_string(),
            };
        }
        if self.can_reduce_reasoning && !self.retry_without_reasoning {
            self.retry_without_reasoning = true;
            eprintln!(
                "PROVIDER_EMPTY_ANSWER_RECOVERY model_id={} next_reasoning=off",
                crate::redaction::redacted_log_text(model_id),
            );
        }
        error
    }
}

pub(super) fn empty(code: &str, message: &str) -> bool {
    if code == "deepseek_reasoning_without_answer" {
        return true;
    }
    code == "provider_response_error"
        && [
            "empty response",
            "no visible text",
            "returned no text",
            "returned no content",
        ]
        .iter()
        .any(|needle| message.to_ascii_lowercase().contains(needle))
}

fn provider_returned_no_visible_answer(error: &InferenceError) -> bool {
    empty(&error.code, &error.message)
}

pub(super) fn execute_remote_chat_inference(
    provider_route: &ResolvedProviderRoute,
    request: InferenceRequest,
    stream: Option<ChatEventStream>,
) -> Result<InferenceResponse, InferenceError> {
    validate_inference_request_attachments(&request)?;
    let retry_stream = stream.clone();
    let mut recovery = EmptyAnswerRecovery::new(&request);
    execute_with_transient_inference_retry(
        "chat_provider_inference",
        || {
            if let Some(stream) = stream.as_ref() {
                stream.reset_emitted_token_count();
            }
            let attempt = recovery.request(&request);
            let result = match stream.clone() {
                Some(stream) => execute_provider_streaming_inference(attempt, stream),
                None => execute_provider_inference(attempt),
            };
            let mut response = result.map_err(|error| recovery.error(error, &request.model_id))?;
            response.provider_id = provider_route.route_provider_id.clone();
            Ok(response)
        },
        |error| retry_allowed_for_stream(error, retry_stream.as_ref()),
    )
}

pub(super) fn execute_provider_inference_with_retry(
    request: InferenceRequest,
    operation_name: &str,
) -> Result<InferenceResponse, InferenceError> {
    validate_inference_request_attachments(&request)?;
    let mut recovery = EmptyAnswerRecovery::new(&request);
    execute_with_transient_inference_retry(
        operation_name,
        || {
            execute_provider_inference(recovery.request(&request))
                .map_err(|error| recovery.error(error, &request.model_id))
        },
        |_| true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::InferenceMessage;

    fn request(provider_id: &str, reasoning: &str) -> InferenceRequest {
        InferenceRequest {
            provider_id: provider_id.to_string(),
            model_id: "deepseek-v4-flash".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
                attachments: Vec::new(),
            }],
            prompt: None,
            temperature: None,
            max_tokens: Some(8_192),
            reasoning: Some(reasoning.to_string()),
            reasoning_budget_tokens: Some(8_000),
            base_url: None,
            api_key_label: None,
            api_key: None,
        }
    }

    #[test]
    fn reasoning_only_recovery_is_provider_specific_and_bounded() {
        let deepseek = request("deepseek", "max");
        let mut recovery = EmptyAnswerRecovery::new(&deepseek);
        assert!(recovery.can_reduce_reasoning);

        let first = recovery.error(
            InferenceError::deepseek_reasoning_without_answer(),
            &deepseek.model_id,
        );
        assert_eq!(first.code, "deepseek_reasoning_without_answer");
        let retry = recovery.request(&deepseek);
        assert_eq!(retry.reasoning.as_deref(), Some("off"));
        assert_eq!(retry.reasoning_budget_tokens, None);

        let second = recovery.error(
            InferenceError::deepseek_reasoning_without_answer(),
            &deepseek.model_id,
        );
        assert_eq!(second.code, "provider_empty_after_recovery");
        assert!(!EmptyAnswerRecovery::new(&request("openai", "max")).can_reduce_reasoning);
        assert!(!EmptyAnswerRecovery::new(&request("deepseek", "off")).can_reduce_reasoning);
    }

    #[test]
    fn gemini_empty_answer_retries_once_with_minimal_reasoning() {
        let gemini = request("google_gemini", "medium");
        let mut recovery = EmptyAnswerRecovery::new(&gemini);
        let error = recovery.error(
            InferenceError::provider(
                "Google Gemini finished normally but returned no visible text.",
            ),
            "gemini-3.5-flash",
        );

        assert_eq!(error.code, "provider_response_error");
        let retry = recovery.request(&gemini);
        assert_eq!(retry.reasoning.as_deref(), Some("off"));
        assert_eq!(retry.reasoning_budget_tokens, None);
    }
}
