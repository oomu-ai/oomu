use base64::{engine::general_purpose, Engine as _};
use oomu_lib::gemma::{
    format_completion_chat_prompt, format_gemma4_chat_prompt, has_repeated_logical_certificate,
    inspect_local_model, local_multimodal_marker, looks_like_reasoning_leak,
    resolve_exact_ready_local_model, sanitize_completion_response, sanitize_gemma4_response,
    strip_leading_reasoning_preamble, GemmaService, GemmaStreamChunk, InferRequest,
    LocalGenerationStream, NativeMediaInput, StructuredLocalInferRequest,
    LOCAL_INFER_PROTOCOL_VERSION, LOCAL_MODEL_DIRECTORY_ENV,
};
use oomu_lib::settings;
use serde::Serialize;
use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process;

const MAX_LOCAL_MEDIA_COUNT: usize = 4;
const MAX_LOCAL_MEDIA_BYTES: usize = 8 * 1024 * 1024;
const MAX_LOCAL_MEDIA_AGGREGATE_BYTES: usize = 20 * 1024 * 1024;

#[derive(Serialize)]
struct LocalInferCliResponse {
    text: String,
    model_path: String,
    service_status: String,
    device: String,
    prompt_token_count: usize,
    generated_token_count: usize,
    inference_latency_ms: u128,
    time_to_first_token_ms: u128,
    trace_hash: String,
    reasoning_trace: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LocalInferCliError {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct LocalInferCliEvent {
    event: &'static str,
    sequence: usize,
    elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "{}",
            serde_json::to_string(&error).unwrap_or_else(|_| {
                "{\"code\":\"local_infer_failed\",\"message\":\"Local inference failed.\"}"
                    .to_string()
            })
        );
        process::exit(1);
    }
}

fn run() -> Result<(), LocalInferCliError> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args
        .first()
        .is_some_and(|value| value == "--protocol-version")
    {
        println!("{LOCAL_INFER_PROTOCOL_VERSION}");
        return Ok(());
    }
    let inspect_only = args.first().is_some_and(|value| value == "--inspect");
    let serve = args.first().is_some_and(|value| value == "--serve");
    let requested_model_id = args
        .get(if inspect_only || serve { 1 } else { 0 })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| oomu_lib::gemma::PREFERRED_LOCAL_MODEL_ID.to_string());
    if inspect_only {
        let manifest =
            inspect_local_model(&requested_model_id).map_err(|error| LocalInferCliError {
                code: error.code,
                message: error.message,
            })?;
        println!(
            "{}",
            serde_json::to_string(&manifest).map_err(|error| LocalInferCliError {
                code: "local_infer_serialize_failed",
                message: format!("Failed to serialize local model manifest: {error}"),
            })?
        );
        return Ok(());
    }

    let model_root = env::var_os(LOCAL_MODEL_DIRECTORY_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(settings::models_root);
    let manifest =
        resolve_exact_ready_local_model(&model_root, &requested_model_id).map_err(|error| {
            LocalInferCliError {
                code: error.code,
                message: error.message,
            }
        })?;
    if manifest.format != "gguf" {
        return Err(LocalInferCliError {
            code: "local_infer_stateful_gguf_required",
            message: format!(
                "Local chat requires a stateful GGUF runtime, but '{}' resolved to {}. Configure or download a quantized GGUF model.",
                manifest.id, manifest.format
            ),
        });
    }
    if manifest.compatibility != "ready" {
        return Err(LocalInferCliError {
            code: "local_model_incompatible",
            message: manifest.compatibility_message,
        });
    }
    let model_id = manifest.id.clone();
    let uses_completion_prompt = manifest.chat_capability != "chat";
    if serve {
        return serve_requests(&model_id, uses_completion_prompt);
    }

    let mut prompt = String::new();
    io::stdin()
        .read_to_string(&mut prompt)
        .map_err(|error| LocalInferCliError {
            code: "local_infer_stdin_failed",
            message: format!("Failed to read prompt from stdin: {error}"),
        })?;
    if prompt.trim().is_empty() {
        return Err(LocalInferCliError {
            code: "local_infer_empty_prompt",
            message: "Local inference requires a non-empty prompt.".to_string(),
        });
    }

    let service = GemmaService::new_loading();
    let output = infer_request(&service, &model_id, uses_completion_prompt, prompt)?;
    service.shutdown();
    write_response(&output)
}

fn serve_requests(model_id: &str, uses_completion_prompt: bool) -> Result<(), LocalInferCliError> {
    let service = GemmaService::new_loading();
    service
        .prepare_model_sync(model_id)
        .map_err(|error| LocalInferCliError {
            code: error.code,
            message: error.message,
        })?;
    let ready_event = serde_json::to_string(&LocalInferCliEvent {
        event: "ready",
        sequence: 0,
        elapsed_ms: 0,
        token: None,
    })
    .map_err(|error| LocalInferCliError {
        code: "local_infer_protocol_serialization_failed",
        message: format!("Failed to serialize the local inference readiness event: {error}"),
    })?;
    eprintln!("{ready_event}");

    for line in BufReader::new(io::stdin().lock()).lines() {
        let prompt = line.map_err(|error| LocalInferCliError {
            code: "local_infer_stdin_failed",
            message: format!("Failed to read prompt from stdin: {error}"),
        })?;
        if prompt.trim().is_empty() {
            continue;
        }
        match infer_request(&service, model_id, uses_completion_prompt, prompt) {
            Ok(output) => write_response(&output)?,
            Err(error) => eprintln!(
                "{}",
                serde_json::to_string(&error).unwrap_or_else(|_| {
                    "{\"code\":\"local_infer_failed\",\"message\":\"Local inference failed.\"}"
                        .to_string()
                })
            ),
        }
    }
    service.shutdown();
    Ok(())
}

fn infer_request(
    service: &GemmaService,
    model_id: &str,
    uses_completion_prompt: bool,
    prompt: String,
) -> Result<LocalInferCliResponse, LocalInferCliError> {
    let mut session_id = None;
    let mut system_prompt = None;
    let mut prompt_is_full_context = false;
    let mut context_size = None;
    let mut max_tokens = None;
    let mut media = Vec::new();
    let prompt = match serde_json::from_str::<StructuredLocalInferRequest>(&prompt) {
        Ok(request) => {
            session_id = request.session_id;
            context_size = request.context_size;
            max_tokens = request.max_tokens;
            prompt_is_full_context = true;
            let mut total_media_bytes = 0usize;
            let mut messages = Vec::with_capacity(request.messages.len());
            for message in request.messages {
                let mut content = message.content;
                if !message.media.is_empty() {
                    if media.len().saturating_add(message.media.len()) > MAX_LOCAL_MEDIA_COUNT {
                        return Err(LocalInferCliError {
                            code: "local_infer_media_count_exceeded",
                            message: "Local image input supports at most four images per turn."
                                .to_string(),
                        });
                    }
                    let mut prefix = String::new();
                    for item in message.media {
                        if !item.mime_type.trim().starts_with("image/") {
                            return Err(LocalInferCliError {
                                code: "local_infer_media_type_unsupported",
                                message:
                                    "The local multimodal route received an unsupported media type."
                                        .to_string(),
                            });
                        }
                        let bytes = general_purpose::STANDARD
                            .decode(item.data_base64.trim())
                            .map_err(|_| LocalInferCliError {
                                code: "local_infer_media_invalid",
                                message: "The approved local image payload was invalid."
                                    .to_string(),
                            })?;
                        if bytes.is_empty() || bytes.len() > MAX_LOCAL_MEDIA_BYTES {
                            return Err(LocalInferCliError {
                                code: "local_infer_media_size_exceeded",
                                message:
                                    "The approved local image exceeds the safe inference limit."
                                        .to_string(),
                            });
                        }
                        total_media_bytes = total_media_bytes.saturating_add(bytes.len());
                        if total_media_bytes > MAX_LOCAL_MEDIA_AGGREGATE_BYTES {
                            return Err(LocalInferCliError {
                                code: "local_infer_media_size_exceeded",
                                message:
                                    "The approved local images exceed the safe inference limit."
                                        .to_string(),
                            });
                        }
                        prefix.push_str(local_multimodal_marker());
                        prefix.push('\n');
                        media.push(NativeMediaInput {
                            name: item.name,
                            mime_type: item.mime_type,
                            bytes,
                        });
                    }
                    prefix.push_str(content.trim_start());
                    content = prefix;
                }
                messages.push((message.role, content));
            }
            let mut effective_system_prompt = request.system_prompt;
            if !media.is_empty() {
                effective_system_prompt.push_str(
                    "\n\nLOCAL IMAGE INPUT\nOOMU has already opened the user-approved image and supplied its pixels directly to you. Analyze the image itself. Do not claim that you cannot access a file path; no path access is required.\nEND LOCAL IMAGE INPUT",
                );
            }
            system_prompt = Some(effective_system_prompt.clone());
            if uses_completion_prompt {
                format_completion_chat_prompt(&effective_system_prompt, &messages)
            } else {
                format_gemma4_chat_prompt(&effective_system_prompt, &messages)
            }
        }
        Err(error) => {
            if prompt.trim_start().starts_with('{') {
                eprintln!("[local_infer] Warning: Failed to deserialize structured prompt JSON payload: {error}");
            }
            prompt
        }
    };

    let mut visible_stream = |chunk: GemmaStreamChunk| {
        let event = if chunk.token.is_empty() {
            progress_event(chunk.sequence, chunk.elapsed_ms)
        } else {
            LocalInferCliEvent {
                event: "token",
                sequence: chunk.sequence,
                elapsed_ms: chunk.elapsed_ms,
                token: Some(chunk.token),
            }
        };
        if let Ok(encoded) = serde_json::to_string(&event) {
            eprintln!("{encoded}");
        }
    };
    let mut progress_stream = |chunk: GemmaStreamChunk| {
        if let Ok(encoded) =
            serde_json::to_string(&progress_event(chunk.sequence, chunk.elapsed_ms))
        {
            eprintln!("{encoded}");
        }
    };
    // These gemma4 checkpoints occasionally return an unusable turn even with a correct prompt:
    //   * an empty visible answer (the model reasons to the end of the turn without ever opening the
    //     visible `<|channel>text` channel, so the sanitizer suppresses everything); or
    //   * on the smallest checkpoint (E2B), a visible chain-of-thought scratchpad ("thinking_level:
    //     ...") despite the output contract.
    // Sampling is seeded randomly, so retry a bounded number of times with a fresh session; a
    // different sample is almost always clean. Keep the best attempt as a fallback, and only as a
    // last resort (every attempt poor) strip the leading reasoning preamble so the user never sees a
    // raw scratchpad header instead of surfacing local_infer_empty_response.
    const MAX_ATTEMPTS: usize = 4;
    let mut fallback: Option<LocalInferCliResponse> = None;
    for attempt in 0..MAX_ATTEMPTS {
        // Reuse the SAME session id across attempts. The runtime keys its KV cache per session and
        // every session shares one llama.cpp sequence, so a per-attempt session id collides on that
        // sequence. Resending the identical prompt under one id instead drives the runtime's
        // regeneration path (it rewinds the final cached token and resamples with a fresh seed).
        //
        // Only stream visible tokens from the first attempt to the client. A retry is triggered by
        // an empty or leaked first attempt, so streaming discarded resamples would flash a
        // scratchpad before the clean answer replaces it; resamples emit progress only.
        let stream_sink: Option<&mut dyn LocalGenerationStream> = if attempt == 0 {
            Some(&mut visible_stream)
        } else {
            Some(&mut progress_stream)
        };
        let response = service
            .infer_model_with_stream_sync(
                &model_id,
                InferRequest {
                    prompt: prompt.clone(),
                    media: media.clone(),
                    session_id: session_id.clone(),
                    system_prompt: system_prompt.clone(),
                    prompt_is_full_context,
                    deterministic: false,
                    context_size,
                    max_tokens,
                    grammar: None,
                    audit_event_kind: None,
                    defer_audit: true,
                    cancellation: Default::default(),
                },
                stream_sink,
            )
            .map_err(|error| LocalInferCliError {
                code: error.code,
                message: error.message,
            })?;

        let has_repeated_certificate =
            !uses_completion_prompt && has_repeated_logical_certificate(&response.text);
        let sanitized_text = sanitize_gemma4_response(&response.text);
        let text = if uses_completion_prompt {
            sanitize_completion_response(&sanitized_text)
        } else {
            sanitized_text
        };
        let is_empty = text.trim().is_empty();
        // Leak detection is a gemma4 chat-channel artifact; never apply it to completion models.
        let is_leak = !uses_completion_prompt && looks_like_reasoning_leak(&text);
        let has_repeated_certificate = has_repeated_certificate
            || (!uses_completion_prompt && has_repeated_logical_certificate(&text));
        let candidate = LocalInferCliResponse {
            text,
            model_path: response.model_path,
            service_status: format!("{:?}", response.service_status),
            device: response.device,
            prompt_token_count: response.prompt_token_count,
            generated_token_count: response.generated_token_count,
            inference_latency_ms: response.inference_latency_ms,
            time_to_first_token_ms: response.time_to_first_token_ms,
            trace_hash: response.trace_hash,
            reasoning_trace: response
                .reasoning_trace
                .into_iter()
                .filter(|entry| !entry.starts_with("Step ") && !contains_control_marker(entry))
                .collect(),
        };

        if !is_empty && !is_leak && !has_repeated_certificate {
            return Ok(candidate);
        }

        // Prefer a non-empty fallback over an empty one; among non-empty candidates prefer the
        // longest, which most likely carries a real answer after the leaked preamble.
        let replace = match &fallback {
            None => true,
            Some(existing) => {
                let existing_empty = existing.text.trim().is_empty();
                (existing_empty && !is_empty)
                    || (existing_empty == is_empty
                        && candidate.text.trim().len() > existing.text.trim().len())
            }
        };
        if replace {
            fallback = Some(candidate);
        }
    }

    let mut final_response = fallback.ok_or_else(|| LocalInferCliError {
        code: "local_infer_retry_exhausted_without_candidate",
        message: "Local inference retry loop ended before producing any candidate response."
            .to_string(),
    })?;
    if !uses_completion_prompt {
        final_response.text = strip_leading_reasoning_preamble(&final_response.text);
    }
    Ok(final_response)
}

fn progress_event(sequence: usize, elapsed_ms: u128) -> LocalInferCliEvent {
    LocalInferCliEvent {
        event: "progress",
        sequence,
        elapsed_ms,
        token: None,
    }
}

fn write_response(output: &LocalInferCliResponse) -> Result<(), LocalInferCliError> {
    println!("{}", serialize_response(output)?);
    io::stdout().flush().map_err(|error| LocalInferCliError {
        code: "local_infer_stdout_failed",
        message: format!("Failed to flush local inference response: {error}"),
    })?;
    Ok(())
}

fn serialize_response(output: &LocalInferCliResponse) -> Result<String, LocalInferCliError> {
    serde_json::to_string(output).map_err(|error| LocalInferCliError {
        code: "local_infer_serialize_failed",
        message: format!("Failed to serialize local inference response: {error}"),
    })
}

fn contains_control_marker(value: &str) -> bool {
    [
        "<|channel>",
        "<channel|>",
        "<|turn>",
        "<turn|>",
        "<think>",
        "<|think|>",
        "<|start_header_id|>",
        "<|end_header_id|>",
        "<|eot_id|>",
    ]
    .iter()
    .any(|marker| value.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::{contains_control_marker, serialize_response, LocalInferCliResponse};

    #[test]
    fn diagnostic_filter_detects_protocol_markers() {
        assert!(contains_control_marker(
            "Step 01: Decoded token '<|channel>'"
        ));
        assert!(contains_control_marker("Unexpected <think> trace"));
        assert!(!contains_control_marker(
            "Transformer loop generated 24 tokens."
        ));
    }

    #[test]
    fn terminal_response_serialization_is_independent_of_audit_and_keychain() {
        let encoded = serialize_response(&LocalInferCliResponse {
            text: "OK".to_string(),
            model_path: "private://local-model/active".to_string(),
            service_status: "Ready".to_string(),
            device: "llama.cpp Metal".to_string(),
            prompt_token_count: 12,
            generated_token_count: 1,
            inference_latency_ms: 25,
            time_to_first_token_ms: 20,
            trace_hash: "trace".to_string(),
            reasoning_trace: vec!["local".to_string()],
        })
        .expect("terminal response serializes without persistence access");

        assert!(encoded.contains(r#""text":"OK""#));
        assert!(encoded.contains(r#""prompt_token_count":12"#));
        assert!(encoded.contains(r#""trace_hash":"trace""#));
    }
}
