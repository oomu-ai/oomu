use super::InferenceError;
use crate::{db::PersistenceEngine, foundation::digest::sha256_hex};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub(super) struct LocalInferTerminal {
    pub text: String,
    pub prompt_token_count: usize,
    pub generated_token_count: usize,
    pub device: String,
    pub inference_latency_ms: u128,
    pub time_to_first_token_ms: u128,
    pub trace_hash: String,
}

#[derive(Debug, Clone)]
pub(super) struct LocalInferenceUsage {
    prompt_token_count: usize,
    generated_token_count: usize,
    device: String,
    inference_latency_ms: u128,
    time_to_first_token_ms: u128,
    trace_hash: String,
    prompt_hash: String,
    output_hash: String,
}

pub(super) fn parse_terminal(line: &str) -> Result<LocalInferTerminal, InferenceError> {
    let mut terminal = serde_json::from_str::<LocalInferTerminal>(line).map_err(|error| {
        InferenceError::local_infer(
            "local_infer_invalid_response",
            format!("Local inference helper returned invalid JSON: {error}"),
        )
    })?;
    terminal.text = terminal.text.trim().to_string();
    if terminal.text.is_empty() {
        return Err(InferenceError::local_infer(
            "local_infer_empty_response",
            "Local inference helper returned an empty response.",
        ));
    }
    Ok(terminal)
}

impl LocalInferenceUsage {
    pub(super) fn from_terminal(prompt: &str, terminal: &LocalInferTerminal) -> Self {
        Self {
            prompt_token_count: terminal.prompt_token_count,
            generated_token_count: terminal.generated_token_count,
            device: terminal.device.clone(),
            inference_latency_ms: terminal.inference_latency_ms,
            time_to_first_token_ms: terminal.time_to_first_token_ms,
            trace_hash: terminal.trace_hash.clone(),
            prompt_hash: sha256_hex(prompt.as_bytes()),
            output_hash: sha256_hex(terminal.text.as_bytes()),
        }
    }

    pub(super) fn refresh_output_hash(&mut self, output: &str) {
        self.output_hash = sha256_hex(output.as_bytes());
    }

    pub(super) fn persist_audit(&self, persistence: &PersistenceEngine) -> rusqlite::Result<()> {
        persistence.insert_local_inference_audit_hashes(
            self.audit_event_kind(),
            &self.prompt_hash,
            &self.output_hash,
            &self.trace_hash,
            &self.device,
            self.inference_latency_ms,
            self.time_to_first_token_ms,
            self.prompt_token_count,
            self.generated_token_count,
        )
    }

    fn audit_event_kind(&self) -> &'static str {
        if self.device == crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_DEVICE {
            crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_EVENT_KIND
        } else {
            "local_gemma_infer"
        }
    }

    pub(super) fn merge_into_metadata(&self, metadata: &mut Value) {
        let Some(metadata) = metadata.as_object_mut() else {
            return;
        };
        metadata.insert("promptTokens".to_string(), json!(self.prompt_token_count));
        metadata.insert(
            "completionTokens".to_string(),
            json!(self.generated_token_count),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_parse_returns_exact_usage_without_persistence() {
        let terminal = parse_terminal(
            r#"{"text":" OK ","prompt_token_count":41,"generated_token_count":2,"device":"Metal","inference_latency_ms":90,"time_to_first_token_ms":70,"trace_hash":"trace"}"#,
        )
        .expect("terminal response parses independently of audit persistence");
        assert_eq!(terminal.text, "OK");

        let usage = LocalInferenceUsage::from_terminal("prompt", &terminal);
        let mut metadata = json!({});
        usage.merge_into_metadata(&mut metadata);
        assert_eq!(metadata["promptTokens"], 41);
        assert_eq!(metadata["completionTokens"], 2);
        assert!(metadata.get("localInferenceUsage").is_none());

        let deterministic = LocalInferenceUsage::from_terminal(
            "rewrite",
            &LocalInferTerminal {
                text: "rewritten".to_string(),
                prompt_token_count: 0,
                generated_token_count: 0,
                device: crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_DEVICE
                    .to_string(),
                inference_latency_ms: 1,
                time_to_first_token_ms: 0,
                trace_hash: "native-transform".to_string(),
            },
        );
        assert_eq!(
            deterministic.audit_event_kind(),
            crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_EVENT_KIND
        );
    }
}
