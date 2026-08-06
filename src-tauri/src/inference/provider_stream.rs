//! Owns bounded remote-provider streaming transport and SSE decoding.

use super::{
    clear_local_stream_cancellation, is_local_stream_cancelled, load_provider_api_key,
    merge_stream_text_chunk, normalize_provider_id, normalize_request, payload_for_provider,
    provider_http_error_message, validate_inference_request_attachments, ChatEventStream,
    InferenceError, InferenceRequest, InferenceResponse, LocalInferToken, ProviderPayload,
};
use futures_util::StreamExt;
use reqwest::Client as AsyncClient;
use serde_json::Value;
use std::time::{Duration, Instant};

const PROVIDER_STREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PROVIDER_STREAM_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const PROVIDER_STREAM_CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const PROVIDER_STREAM_MAX_DURATION: Duration = Duration::from_secs(15 * 60);
const MAX_PROVIDER_STREAM_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_PROVIDER_SSE_PENDING_EVENT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_PROVIDER_RESPONSE_TEXT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct ProviderStreamState {
    text: String,
    response_id: Option<String>,
    finish_reason: Option<String>,
    empty_response_message: Option<String>,
    reasoning_observed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderStreamTimeoutPolicy {
    pub(super) connect_timeout: Duration,
    pub(super) read_idle_timeout: Duration,
}

fn provider_stream_timeout_policy() -> ProviderStreamTimeoutPolicy {
    ProviderStreamTimeoutPolicy {
        connect_timeout: PROVIDER_STREAM_CONNECT_TIMEOUT,
        read_idle_timeout: PROVIDER_STREAM_READ_IDLE_TIMEOUT,
    }
}

pub(super) fn apply_provider_stream_timeout_policy(
    builder: reqwest::ClientBuilder,
    policy: ProviderStreamTimeoutPolicy,
) -> reqwest::ClientBuilder {
    builder
        .connect_timeout(policy.connect_timeout)
        .read_timeout(policy.read_idle_timeout)
}

fn hardened_provider_async_client_builder() -> reqwest::ClientBuilder {
    apply_provider_stream_timeout_policy(AsyncClient::builder(), provider_stream_timeout_policy())
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
}

pub(super) fn execute_provider_streaming_inference(
    request: InferenceRequest,
    stream: ChatEventStream,
) -> Result<InferenceResponse, InferenceError> {
    validate_inference_request_attachments(&request)?;
    let call = execute_provider_streaming_inference_async(request, stream);
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(call),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| InferenceError::worker(error.to_string()))?
            .block_on(call),
    }
}

async fn execute_provider_streaming_inference_async(
    request: InferenceRequest,
    stream: ChatEventStream,
) -> Result<InferenceResponse, InferenceError> {
    let stream_id = stream.stream_id.clone();
    let result = async {
        let provider_id =
            normalize_provider_id(&request.provider_id).map_err(InferenceError::invalid)?;
        let provider = payload_for_provider(&provider_id)?;
        let api_key_label = request.api_key_label.clone();
        let configured_api_key = request.api_key.clone();
        let api_key = load_provider_api_key(
            &provider_id,
            api_key_label.as_deref(),
            configured_api_key.as_deref(),
        )?;
        let http_request = normalize_request(request)?;
        let client = hardened_provider_async_client_builder()
            .build()
            .map_err(|error| InferenceError::network(error.to_string()))?;

        ensure_remote_stream_active(&stream_id)?;
        let started = Instant::now();
        let stream_deadline = tokio::time::sleep(PROVIDER_STREAM_MAX_DURATION);
        tokio::pin!(stream_deadline);
        let request = provider.build_stream_request(&client, &api_key, &http_request)?;
        let response = tokio::select! {
            response = request.send() => response,
            _ = wait_for_remote_stream_cancellation(&stream_id) => {
                return Err(remote_stream_cancelled_error());
            }
            _ = &mut stream_deadline => {
                return Err(provider_stream_duration_exceeded_error());
            }
        }
        .and_then(|response| response.error_for_status())
        .map_err(InferenceError::network_from_reqwest)?;
        let mut byte_stream = response.bytes_stream();
        let mut decoder = SseEventDecoder::default();
        let mut state = ProviderStreamState::default();
        let mut sequence = 0usize;
        let mut received_stream_bytes = 0usize;

        loop {
            let chunk = tokio::select! {
                chunk = byte_stream.next() => chunk,
                _ = wait_for_remote_stream_cancellation(&stream_id) => {
                    return Err(remote_stream_cancelled_error());
                }
                _ = &mut stream_deadline => {
                    return Err(provider_stream_duration_exceeded_error());
                }
            };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) if sequence > 0 => {
                    let interruption = provider_stream_interruption_message(&error, sequence);
                    eprintln!(
                        "PROVIDER_STREAM_INTERRUPTED provider={} model_id={} stream_events={} elapsed_ms={} timeout={} connect={} decode={} detail={}",
                        provider.provider_name(),
                        http_request.model_id,
                        sequence,
                        started.elapsed().as_millis(),
                        error.is_timeout(),
                        error.is_connect(),
                        error.is_decode(),
                        provider_http_error_message(&error)
                    );
                    return Err(InferenceError::provider_stream_interrupted_after_tokens(
                        interruption,
                    ));
                }
                Err(error) => return Err(InferenceError::network(error.to_string())),
            };
            received_stream_bytes = received_stream_bytes
                .checked_add(chunk.len())
                .ok_or_else(|| {
                    InferenceError::provider(
                        "The remote provider stream exceeded OOMU's response safety limit.",
                    )
                })?;
            if received_stream_bytes > MAX_PROVIDER_STREAM_BYTES {
                return Err(InferenceError::provider(format!(
                    "The remote provider stream exceeded OOMU's {} MiB response safety limit.",
                    MAX_PROVIDER_STREAM_BYTES / (1024 * 1024)
                )));
            }
            for event_data in decoder
                .push_chunk(&chunk)
                .map_err(|error| InferenceError::provider(error.to_string()))?
            {
                if process_provider_stream_event(
                    provider.as_ref(),
                    &event_data,
                    &stream,
                    &mut sequence,
                    &mut state,
                )? {
                    return stream_response(
                        provider.as_ref(),
                        provider_id,
                        http_request.model_id,
                        state,
                        started,
                    );
                }
            }
        }

        let mut reached_terminal_event = false;
        for event_data in decoder
            .finish()
            .map_err(|error| InferenceError::provider(error.to_string()))?
        {
            if process_provider_stream_event(
                provider.as_ref(),
                &event_data,
                &stream,
                &mut sequence,
                &mut state,
            )? {
                reached_terminal_event = true;
                break;
            }
        }

        if !reached_terminal_event {
            return Err(provider_stream_ended_before_terminal_error(sequence));
        }

        stream_response(
            provider.as_ref(),
            provider_id,
            http_request.model_id,
            state,
            started,
        )
    }
    .await;
    clear_local_stream_cancellation(&stream_id);
    result
}

fn ensure_remote_stream_active(stream_id: &str) -> Result<(), InferenceError> {
    if is_local_stream_cancelled(Some(stream_id)) {
        return Err(remote_stream_cancelled_error());
    }
    Ok(())
}

async fn wait_for_remote_stream_cancellation(stream_id: &str) {
    loop {
        if is_local_stream_cancelled(Some(stream_id)) {
            return;
        }
        tokio::time::sleep(PROVIDER_STREAM_CANCELLATION_POLL_INTERVAL).await;
    }
}

fn remote_stream_cancelled_error() -> InferenceError {
    InferenceError::local_infer(
        "local_inference_cancelled",
        "Remote generation was cancelled.",
    )
}

pub(super) fn provider_stream_duration_exceeded_error() -> InferenceError {
    InferenceError {
        code: "provider_stream_duration_exceeded".to_string(),
        boundary: "provider_api".to_string(),
        message: format!(
            "The remote provider did not finish within OOMU's {}-minute response safety limit. The incomplete response was withheld.",
            PROVIDER_STREAM_MAX_DURATION.as_secs() / 60
        ),
    }
}

pub(super) fn provider_stream_ended_before_terminal_error(sequence: usize) -> InferenceError {
    if sequence > 0 {
        return InferenceError::provider_stream_interrupted_after_tokens(format!(
            "The remote provider ended the connection before confirming that the response was complete after OOMU received {sequence} stream event(s). The incomplete response was withheld so OOMU could safely retry the same provider and model."
        ));
    }
    InferenceError::network(
        "The remote provider ended the connection before returning a complete response.",
    )
}

fn provider_stream_interruption_message(error: &reqwest::Error, sequence: usize) -> String {
    if error.is_timeout() {
        return format!(
            "The remote provider stopped sending data for {} seconds after OOMU received {sequence} stream event(s). The incomplete response was withheld so OOMU could safely retry the same provider and model.",
            PROVIDER_STREAM_READ_IDLE_TIMEOUT.as_secs()
        );
    }
    format!(
        "The remote provider connection closed before the response finished after OOMU received {sequence} stream event(s). The incomplete response was withheld so OOMU could safely retry the same provider and model."
    )
}

#[derive(Default)]
pub(super) struct SseEventDecoder {
    buffer: Vec<u8>,
}

impl SseEventDecoder {
    pub(super) fn push_chunk(&mut self, chunk: impl AsRef<[u8]>) -> Result<Vec<String>, String> {
        self.buffer.extend_from_slice(chunk.as_ref());
        let events = self.drain_events(false)?;
        if self.buffer.len() > MAX_PROVIDER_SSE_PENDING_EVENT_BYTES {
            return Err(format!(
                "The remote provider sent an SSE event larger than OOMU's {} MiB safety limit.",
                MAX_PROVIDER_SSE_PENDING_EVENT_BYTES / (1024 * 1024)
            ));
        }
        Ok(events)
    }

    pub(super) fn finish(&mut self) -> Result<Vec<String>, String> {
        self.drain_events(true)
    }

    fn drain_events(&mut self, finish: bool) -> Result<Vec<String>, String> {
        let mut events = Vec::new();
        let mut consumed_bytes = 0usize;
        while let Some(event_bytes) = sse_event_boundary(&self.buffer[consumed_bytes..]) {
            if event_bytes > MAX_PROVIDER_SSE_PENDING_EVENT_BYTES {
                return Err(format!(
                    "The remote provider sent an SSE event larger than OOMU's {} MiB safety limit.",
                    MAX_PROVIDER_SSE_PENDING_EVENT_BYTES / (1024 * 1024)
                ));
            }
            let event_end = consumed_bytes + event_bytes;
            let raw = String::from_utf8(self.buffer[consumed_bytes..event_end].to_vec())
                .map_err(|_| "The remote provider stream contained invalid UTF-8.".to_string())?;
            let raw = if raw.as_bytes().contains(&b'\r') {
                raw.replace("\r\n", "\n").replace('\r', "\n")
            } else {
                raw
            };
            if let Some(data) = sse_event_data(&raw) {
                events.push(data);
            }
            consumed_bytes = event_end;
        }
        if consumed_bytes > 0 {
            self.buffer.drain(..consumed_bytes);
        }
        if finish && self.buffer.iter().any(|byte| !byte.is_ascii_whitespace()) {
            let raw = std::mem::take(&mut self.buffer);
            let raw = String::from_utf8(raw)
                .map_err(|_| "The remote provider stream contained invalid UTF-8.".to_string())?;
            let raw = if raw.as_bytes().contains(&b'\r') {
                raw.replace("\r\n", "\n").replace('\r', "\n")
            } else {
                raw
            };
            if let Some(data) = sse_event_data(&raw) {
                events.push(data);
            }
        }
        Ok(events)
    }
}

fn sse_event_boundary(buffer: &[u8]) -> Option<usize> {
    for index in 0..buffer.len() {
        let Some(first) = sse_line_ending_len(buffer, index) else {
            continue;
        };
        let second_index = index + first;
        if let Some(second) = sse_line_ending_len(buffer, second_index) {
            return Some(second_index + second);
        }
    }
    None
}

fn sse_line_ending_len(buffer: &[u8], index: usize) -> Option<usize> {
    match buffer.get(index).copied() {
        Some(b'\n') => Some(1),
        Some(b'\r') if buffer.get(index + 1) == Some(&b'\n') => Some(2),
        Some(b'\r') => Some(1),
        _ => None,
    }
}

fn sse_event_data(raw: &str) -> Option<String> {
    let mut data_lines = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.strip_prefix(' ').unwrap_or(value).to_string());
        }
    }
    if data_lines.is_empty() {
        let trimmed = raw.trim();
        if trimmed.starts_with('{') || trimmed == "[DONE]" {
            return Some(trimmed.to_string());
        }
        return None;
    }
    Some(data_lines.join("\n"))
}

fn process_provider_stream_event(
    provider: &dyn ProviderPayload,
    event_data: &str,
    stream: &ChatEventStream,
    sequence: &mut usize,
    state: &mut ProviderStreamState,
) -> Result<bool, InferenceError> {
    let event_data = event_data.trim();
    if event_data.is_empty() {
        return Ok(false);
    }
    if event_data.eq_ignore_ascii_case("[DONE]") {
        return Ok(true);
    }

    let value = serde_json::from_str::<Value>(event_data)
        .map_err(|error| InferenceError::provider(error.to_string()))?;
    let event = provider.parse_stream_event(&value);
    state.reasoning_observed |= event.reasoning_observed;
    if state.response_id.is_none() {
        state.response_id = event.response_id;
    }
    let should_finish = event.finish_reason.is_some();
    if state.finish_reason.is_none() {
        state.finish_reason = event.finish_reason;
    }
    if state.empty_response_message.is_none() {
        state.empty_response_message = event.empty_response_message;
    }

    if let Some(token) = event.token.filter(|token| !token.is_empty()) {
        let token = merge_stream_text_chunk(&state.text, &token);
        ensure_provider_response_text_capacity(state.text.len(), token.len())?;
        *sequence += 1;
        state.text.push_str(&token);
        stream.emit(LocalInferToken {
            sequence: *sequence,
            token,
        });
    }
    Ok(should_finish)
}

pub(super) fn ensure_provider_response_text_capacity(
    current_bytes: usize,
    additional_bytes: usize,
) -> Result<(), InferenceError> {
    let next_text_bytes = current_bytes.checked_add(additional_bytes).ok_or_else(|| {
        InferenceError::provider("The remote provider response exceeded OOMU's text safety limit.")
    })?;
    if next_text_bytes > MAX_PROVIDER_RESPONSE_TEXT_BYTES {
        return Err(InferenceError::provider(format!(
            "The remote provider response exceeded OOMU's {} MiB text safety limit.",
            MAX_PROVIDER_RESPONSE_TEXT_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

fn stream_response(
    provider: &dyn ProviderPayload,
    provider_id: String,
    model_id: String,
    state: ProviderStreamState,
    started: Instant,
) -> Result<InferenceResponse, InferenceError> {
    let text = state.text.trim().to_string();
    if text.is_empty() {
        if provider.provider_name() == "DeepSeek" && state.reasoning_observed {
            return Err(InferenceError::deepseek_reasoning_without_answer());
        }
        return Err(InferenceError::provider(
            state
                .empty_response_message
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| provider.empty_response_message(state.finish_reason.as_deref())),
        ));
    }
    Ok(InferenceResponse {
        provider_id,
        provider: provider.provider_name().to_string(),
        model_id,
        text,
        response_id: state.response_id,
        finish_reason: state.finish_reason,
        latency_ms: started.elapsed().as_millis(),
        local_usage: None,
    })
}
