use super::LocalInferToken;
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::Emitter;

const DELIVERY_MIN_DURATION_MS: u64 = 750;
const DELIVERY_MIN_PACE_MS: u64 = 20;
const DELIVERY_MAX_PACE_MS: u64 = 400;

const TARGET_CHARS: usize = 72;
const MAX_CHARS: usize = 112;
#[derive(Clone)]
pub(super) struct ChatEventStream {
    app: tauri::AppHandle,
    pub(super) stream_id: String,
    session_id: String,
    turn_id: String,
    generation_token: String,
    emitted_tokens: Arc<AtomicUsize>,
}

impl ChatEventStream {
    pub(super) fn new(
        app: tauri::AppHandle,
        stream_id: &str,
        session_id: String,
        turn_id: String,
        generation_token: String,
    ) -> Self {
        Self {
            app,
            stream_id: stream_id.to_string(),
            session_id,
            turn_id,
            generation_token,
            emitted_tokens: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn emit(&self, token: LocalInferToken) {
        if !token.token.is_empty() {
            self.emitted_tokens
                .fetch_max(token.sequence, Ordering::Relaxed);
        }
    }

    pub(super) fn emit_validated_text(
        self,
        text: &str,
        executing_provider_id: &str,
        executing_model_id: &str,
    ) {
        match deliver(
            &self.app,
            &self.stream_id,
            &self.session_id,
            &self.turn_id,
            &self.generation_token,
            text,
        ) {
            Ok(Some(receipt)) => {
                crate::diagnostic_output::write_functional_acceptance_receipt(&native_receipt(
                    &self.session_id,
                    &self.turn_id,
                    &self.generation_token,
                    executing_provider_id,
                    executing_model_id,
                    &receipt,
                ))
            }
            Ok(None) => {}
            Err(error) => crate::diagnostic_output::write_diagnostic_line(format_args!(
                "CHAT_VALIDATED_STREAM_DELIVERY_FAILED {}",
                crate::redaction::redacted_log_text(&error)
            )),
        }
    }

    pub(super) fn emitted_token_count(&self) -> usize {
        self.emitted_tokens.load(Ordering::Relaxed)
    }

    pub(super) fn reset_emitted_token_count(&self) {
        self.emitted_tokens.store(0, Ordering::Relaxed);
    }
}

pub(super) fn split_handles<T: Clone>(
    accepted_response: Option<T>,
    _buffer_provisional_response: bool,
) -> (Option<T>, Option<T>) {
    // ChatEventStream carries the cancellation identity as well as provisional
    // token accounting. Keeping it attached does not expose unvalidated text:
    // ChatEventStream::emit only counts provisional tokens, while
    // emit_validated_text performs the sole renderer delivery.
    (accepted_response.clone(), accepted_response)
}

#[derive(Clone, Debug, Serialize)]
struct ChatTokenEvent<'a> {
    stream_id: &'a str,
    session_id: &'a str,
    turn_id: &'a str,
    generation_token: &'a str,
    sequence: usize,
    token: String,
    elapsed_ms: u128,
    delivery_state: &'a str,
}

#[derive(Clone, Debug, Serialize)]
struct ValidatedStreamCompleteEvent<'a> {
    stream_id: &'a str,
    session_id: &'a str,
    turn_id: &'a str,
    generation_token: &'a str,
    last_sequence: usize,
    chunk_count: usize,
    text_sha256: &'a str,
    delivery_state: &'a str,
}

pub(super) struct DeliveryReceipt {
    pub chunk_count: usize,
    pub text_sha256: String,
}

fn delivery_receipt(text: &str, chunk_count: usize) -> DeliveryReceipt {
    DeliveryReceipt {
        chunk_count,
        text_sha256: crate::foundation::digest::sha256_hex(text.as_bytes()),
    }
}

pub(super) fn native_receipt(
    session_id: &str,
    turn_id: &str,
    generation_token: &str,
    executing_provider_id: &str,
    executing_model_id: &str,
    receipt: &DeliveryReceipt,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "validated_chat_stream", "sessionId": session_id, "turnId": turn_id,
        "generationToken": generation_token, "executingProviderId": executing_provider_id,
        "executingModelId": executing_model_id, "chunkCount": receipt.chunk_count,
        "textSha256": receipt.text_sha256,
    })
}

pub(super) fn deliver(
    app: &tauri::AppHandle,
    stream_id: &str,
    session_id: &str,
    turn_id: &str,
    generation_token: &str,
    text: &str,
) -> Result<Option<DeliveryReceipt>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let chunks = response_chunks(text);
    let pace_ms = delivery_pace_ms(chunks.len());
    let started = Instant::now();
    for (index, token) in chunks.iter().enumerate() {
        app.emit(
            "chat://token",
            ChatTokenEvent {
                stream_id,
                session_id,
                turn_id,
                generation_token,
                sequence: index + 1,
                token: token.clone(),
                elapsed_ms: started.elapsed().as_millis(),
                delivery_state: "validated",
            },
        )
        .map_err(|error| error.to_string())?;
        if index + 1 < chunks.len() {
            std::thread::sleep(Duration::from_millis(pace_ms));
        }
    }
    let receipt = delivery_receipt(text, chunks.len());
    app.emit(
        "chat://validated-stream-complete",
        ValidatedStreamCompleteEvent {
            stream_id,
            session_id,
            turn_id,
            generation_token,
            last_sequence: chunks.len(),
            chunk_count: receipt.chunk_count,
            text_sha256: &receipt.text_sha256,
            delivery_state: "validated",
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(Some(receipt))
}

fn delivery_pace_ms(chunk_count: usize) -> u64 {
    let gaps = chunk_count.saturating_sub(1) as u64;
    if gaps == 0 {
        return 0;
    }
    DELIVERY_MIN_DURATION_MS
        .div_ceil(gaps)
        .clamp(DELIVERY_MIN_PACE_MS, DELIVERY_MAX_PACE_MS)
}

pub(super) fn response_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut chars = 0;

    for (index, character) in text.char_indices() {
        chars += 1;
        let natural_boundary = character.is_whitespace()
            || matches!(character, '.' | ',' | ';' | ':' | '!' | '?' | '\n');
        if chars >= MAX_CHARS || chars >= TARGET_CHARS && natural_boundary {
            let end = index + character.len_utf8();
            chunks.push(text[start..end].to_string());
            start = end;
            chars = 0;
        }
    }

    if start < text.len() {
        chunks.push(text[start..].to_string());
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_preserves_exact_order_unicode_and_edge_whitespace() {
        let response = "Rust and Node.js each have an official release history. ".repeat(8);
        assert_eq!(response_chunks(&response).concat(), response);
        let short = "Verified — 完了.";
        assert_eq!(response_chunks(short), vec![short]);
        let exact = "\n  Résumé 日本語 🚀 evidence.  \n";
        assert_eq!(response_chunks(exact).concat(), exact);
    }

    #[test]
    fn validated_stream_pacing_is_visible_without_penalizing_long_answers() {
        assert_eq!(delivery_pace_ms(1), 0);
        assert_eq!(delivery_pace_ms(2), 400);
        assert_eq!(delivery_pace_ms(3), 375);
        assert_eq!(delivery_pace_ms(100), 20);
    }

    #[test]
    fn terminal_payload_binds_identity_sequence_and_exact_text_digest() {
        let text = "\n accepted bytes \n";
        let exact_digest = crate::foundation::digest::sha256_hex(text.as_bytes());
        let trimmed_digest = crate::foundation::digest::sha256_hex(text.trim().as_bytes());
        let receipt = delivery_receipt(text, 3);
        let event = ValidatedStreamCompleteEvent {
            stream_id: "stream-1",
            session_id: "session-1",
            turn_id: "turn-1",
            generation_token: "generation-1",
            last_sequence: 3,
            chunk_count: 3,
            text_sha256: &receipt.text_sha256,
            delivery_state: "validated",
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["stream_id"], "stream-1");
        assert_eq!(value["session_id"], "session-1");
        assert_eq!(value["turn_id"], "turn-1");
        assert_eq!(value["generation_token"], "generation-1");
        assert_eq!(value["last_sequence"], 3);
        assert_eq!(value["chunk_count"], 3);
        assert_eq!(value["text_sha256"], exact_digest);
        assert_ne!(value["text_sha256"], trimmed_digest);
        assert_eq!(value["delivery_state"], "validated");
    }

    #[test]
    fn native_receipt_binds_validated_text_to_actual_execution_identity() {
        let delivered = delivery_receipt("accepted", 3);
        let receipt = native_receipt("s", "t", "g", "p", "m", &delivered);
        assert_eq!(receipt["executingProviderId"], "p");
        assert_eq!(receipt["executingModelId"], "m");
        assert_eq!(receipt["turnId"], "t");
        assert_eq!(receipt["generationToken"], "g");
    }

    #[test]
    fn sprint_304_buffered_validation_preserves_the_cancellable_transport_identity() {
        let (transport, validated) = split_handles(Some("stream-304"), true);
        assert_eq!(transport, Some("stream-304"));
        assert_eq!(validated, Some("stream-304"));
    }
}
