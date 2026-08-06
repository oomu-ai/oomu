//! Provider-specific recovery for DeepSeek reasoning-only completions.

use super::{
    execute_provider_inference, execute_provider_streaming_inference,
    execute_with_transient_inference_retry, retry_allowed_for_stream,
    validate_inference_request_attachments, ChatEventStream, InferenceError, InferenceRequest,
    InferenceResponse, ResolvedProviderRoute,
};

struct DeepSeekReasoningRecovery {
    allowed: bool,
    retry_without_reasoning: bool,
}

impl DeepSeekReasoningRecovery {
    fn new(request: &InferenceRequest) -> Self {
        let allowed = matches!(
            request.provider_id.trim().to_ascii_lowercase().as_str(),
            "deepseek" | "deepseek_v3" | "deepseek_r1"
        ) && request
            .reasoning
            .as_deref()
            .map(str::trim)
            .is_some_and(|reasoning| {
                !reasoning.is_empty()
                    && !matches!(
                        reasoning.to_ascii_lowercase().as_str(),
                        "off" | "none" | "disabled" | "false" | "0"
                    )
            });
        Self {
            allowed,
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
        if error.code != "deepseek_reasoning_without_answer" {
            return error;
        }
        if self.allowed && !self.retry_without_reasoning {
            self.retry_without_reasoning = true;
            eprintln!(
                "DEEPSEEK_REASONING_ONLY_RECOVERY model_id={} next_reasoning=off",
                crate::redaction::redacted_log_text(model_id),
            );
            return error;
        }
        InferenceError::provider("DeepSeek returned an empty response.")
    }
}

pub(super) fn execute_remote_chat_inference(
    provider_route: &ResolvedProviderRoute,
    request: InferenceRequest,
    stream: Option<ChatEventStream>,
) -> Result<InferenceResponse, InferenceError> {
    validate_inference_request_attachments(&request)?;
    let retry_stream = stream.clone();
    let mut recovery = DeepSeekReasoningRecovery::new(&request);
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
    let mut recovery = DeepSeekReasoningRecovery::new(&request);
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
        let mut recovery = DeepSeekReasoningRecovery::new(&deepseek);
        assert!(recovery.allowed);

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
        assert_eq!(second.code, "provider_response_error");
        assert!(!DeepSeekReasoningRecovery::new(&request("openai", "max")).allowed);
        assert!(!DeepSeekReasoningRecovery::new(&request("deepseek", "off")).allowed);
    }
}
