use super::*;

pub(crate) const BOUNDED_REWRITE_TRANSFORM_EVENT_KIND: &str = "local_deterministic_bounded_rewrite";
pub(crate) const BOUNDED_REWRITE_TRANSFORM_MODEL_PATH: &str = "native://bounded-rewrite-v1";
pub(crate) const BOUNDED_REWRITE_TRANSFORM_DEVICE: &str = "Native deterministic transform";

pub(crate) fn has_bounded_exact_rewrite_contract(prompt: &str) -> bool {
    integrity::bounded_rewrite_response(prompt).is_some()
}

pub(super) fn can_execute_without_model(request: &InferRequest) -> bool {
    request.media.is_empty() && has_bounded_exact_rewrite_contract(&request.prompt)
}

impl GemmaService {
    pub(super) fn deterministic_transform_preflight(
        &self,
        request: &InferRequest,
        started: Instant,
    ) -> Result<Option<InferResponse>, GemmaError> {
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            return Err(GemmaError {
                code: "gemma_empty_prompt",
                message: "infer requires a non-empty prompt.".to_string(),
            });
        }
        if !request.media.is_empty() {
            return Ok(None);
        }
        let Some(text) = integrity::bounded_rewrite_response(prompt) else {
            return Ok(None);
        };
        if request.cancellation.load(Ordering::Acquire) {
            return Err(GemmaError {
                code: "local_inference_cancelled",
                message: "Local inference was cancelled before completion.".to_string(),
            });
        }

        let reasoning_trace = vec![
            "Matched the native bounded-rewrite contract for the latest user turn.".to_string(),
            "Applied the requested lexical replacement without loading model weights, generating transformer tokens, streaming model output, or contacting a network provider."
                .to_string(),
        ];
        let trace_hash = sha256_hex(
            serde_json::to_string(&reasoning_trace)
                .unwrap_or_default()
                .as_bytes(),
        );
        let inference_latency_ms = started.elapsed().as_millis();
        if should_log_local_inference_audit(request) {
            let persistence = self
                .audit_persistence
                .lock()
                .ok()
                .and_then(|attached| attached.clone());
            if let Some(persistence) = persistence {
                if let Err(error) = persistence.insert_local_inference_audit(
                    BOUNDED_REWRITE_TRANSFORM_EVENT_KIND,
                    prompt,
                    &text,
                    &trace_hash,
                    BOUNDED_REWRITE_TRANSFORM_DEVICE,
                    inference_latency_ms,
                    0,
                    0,
                    0,
                ) {
                    eprintln!(
                        "LOCAL_INFERENCE_AUDIT_FAILED event_kind={} error={}",
                        BOUNDED_REWRITE_TRANSFORM_EVENT_KIND,
                        crate::redaction::redacted_log_text(&error.to_string())
                    );
                }
            }
        }

        Ok(Some(InferResponse {
            token: String::new(),
            text,
            prompt_token_count: 0,
            generated_token_count: 0,
            network_latency_ms: 0,
            inference_latency_ms,
            time_to_first_token_ms: 0,
            service_status: self.get_status(),
            model_path: BOUNDED_REWRITE_TRANSFORM_MODEL_PATH.to_string(),
            device: BOUNDED_REWRITE_TRANSFORM_DEVICE.to_string(),
            trace_hash,
            reasoning_trace,
        }))
    }
}
