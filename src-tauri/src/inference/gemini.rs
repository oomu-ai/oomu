use reqwest::blocking::{Client as BlockingClient, RequestBuilder as BlockingRequestBuilder};
use reqwest::{Client as AsyncClient, RequestBuilder as AsyncRequestBuilder};
use serde_json::{json, Value};

use super::{
    require_https_url, InferenceError, InferenceMessage, ProviderHttpRequest, ProviderPayload,
    ProviderResponse, ProviderStreamEvent,
};

const GEMINI_ENDPOINT: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent";
const GEMINI_STREAM_ENDPOINT: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent";

pub struct GeminiPayload;

impl ProviderPayload for GeminiPayload {
    fn provider_name(&self) -> &'static str {
        "Google Gemini"
    }

    fn build_request(
        &self,
        client: &BlockingClient,
        api_key: &str,
        request: &ProviderHttpRequest,
    ) -> Result<BlockingRequestBuilder, InferenceError> {
        let endpoint = GEMINI_ENDPOINT.replace("{model}", &request.model_id);
        require_https_url(&endpoint)?;
        let body = gemini_body(request)?;

        Ok(client.post(endpoint).query(&[("key", api_key)]).json(&body))
    }

    fn build_stream_request(
        &self,
        client: &AsyncClient,
        api_key: &str,
        request: &ProviderHttpRequest,
    ) -> Result<AsyncRequestBuilder, InferenceError> {
        let endpoint = GEMINI_STREAM_ENDPOINT.replace("{model}", &request.model_id);
        require_https_url(&endpoint)?;
        let body = gemini_body(request)?;

        Ok(client
            .post(endpoint)
            .query(&[("key", api_key), ("alt", "sse")])
            .json(&body))
    }

    fn parse_response(&self, value: Value) -> Result<ProviderResponse, InferenceError> {
        let text = gemini_text(&value);
        let finish_reason = gemini_finish_reason(&value);
        log_gemini_response_metadata(&value, text.chars().count());
        if text.is_empty() {
            return Err(InferenceError::provider(gemini_empty_response_message(
                Some(&value),
                finish_reason.as_deref(),
            )));
        }

        Ok(ProviderResponse {
            text,
            response_id: None,
            finish_reason,
        })
    }

    fn parse_stream_event(&self, value: &Value) -> ProviderStreamEvent {
        let token = gemini_text(value);
        let finish_reason = gemini_finish_reason(value);
        if finish_reason.is_some() {
            log_gemini_response_metadata(value, token.chars().count());
        }
        let empty_response_message = (token.is_empty() && finish_reason.is_some())
            .then(|| gemini_empty_response_message(Some(value), finish_reason.as_deref()));
        ProviderStreamEvent {
            token: (!token.is_empty()).then_some(token),
            reasoning_observed: false,
            response_id: None,
            finish_reason,
            empty_response_message,
        }
    }

    fn empty_response_message(&self, finish_reason: Option<&str>) -> String {
        gemini_empty_response_message(None, finish_reason)
    }
}

fn gemini_text(value: &Value) -> String {
    gemini_text_parts(value)
        .into_iter()
        .collect::<String>()
        .trim()
        .to_string()
}

fn gemini_text_parts(value: &Value) -> Vec<&str> {
    value
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(gemini_visible_text_part)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn gemini_visible_text_part(part: &Value) -> Option<&str> {
    if part.get("thought").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    part.get("text").and_then(Value::as_str)
}

fn gemini_finish_reason(value: &Value) -> Option<String> {
    value
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn log_gemini_response_metadata(value: &Value, text_chars: usize) {
    let candidate_count = value
        .get("candidates")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let text_part_count = gemini_text_parts(value).len();
    let finish_reason = gemini_finish_reason(value).unwrap_or_else(|| "none".to_string());
    let prompt_block_reason = value
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let safety_ratings = gemini_safety_ratings(value, "/candidates/0/safetyRatings")
        .unwrap_or_else(|| "none".to_string());

    eprintln!(
        "GEMINI_RESPONSE_METADATA candidate_count={} finish_reason={} text_part_count={} text_chars={} safety_ratings={} prompt_block_reason={}",
        candidate_count,
        log_field(&finish_reason),
        text_part_count,
        text_chars,
        safety_ratings,
        log_field(prompt_block_reason)
    );
}

fn gemini_empty_response_message(value: Option<&Value>, finish_reason: Option<&str>) -> String {
    let finish_reason = finish_reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty());
    let prompt_block_reason = value
        .and_then(|value| value.pointer("/promptFeedback/blockReason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|reason| !reason.is_empty());

    let mut details = Vec::new();
    if let Some(reason) = finish_reason {
        details.push(format!("finishReason={}", log_field(reason)));
    }
    if let Some(reason) = prompt_block_reason {
        details.push(format!("promptBlockReason={}", log_field(reason)));
    }
    if let Some(value) = value {
        let part_kinds = gemini_part_kinds(value);
        if !part_kinds.is_empty() {
            details.push(format!("partKinds={}", part_kinds.join(",")));
        }
        if let Some(ratings) = gemini_safety_ratings(value, "/candidates/0/safetyRatings") {
            details.push(format!("candidateSafety={ratings}"));
        }
        if let Some(ratings) = gemini_safety_ratings(value, "/promptFeedback/safetyRatings") {
            details.push(format!("promptSafety={ratings}"));
        }
    }

    let mut message = match (finish_reason, prompt_block_reason) {
        (_, Some(_)) => "Google Gemini blocked the prompt before returning visible text.".to_string(),
        (Some(reason), _) if reason.eq_ignore_ascii_case("MAX_TOKENS") => {
            "Google Gemini produced no visible text before reaching MAX_TOKENS; increase the output token limit or lower reasoning.".to_string()
        }
        (Some(reason), _) if reason.eq_ignore_ascii_case("SAFETY") => {
            "Google Gemini blocked the candidate before returning visible text.".to_string()
        }
        (Some(reason), _) if reason.eq_ignore_ascii_case("RECITATION") => {
            "Google Gemini stopped for RECITATION before returning visible text.".to_string()
        }
        (Some(reason), _)
            if matches!(
                reason,
                "MALFORMED_FUNCTION_CALL"
                    | "UNEXPECTED_TOOL_CALL"
                    | "TOO_MANY_TOOL_CALLS"
                    | "MISSING_THOUGHT_SIGNATURE"
                    | "MALFORMED_RESPONSE"
            ) =>
        {
            format!("Google Gemini stopped with {reason} before returning visible text.")
        }
        (Some(reason), _) if reason.eq_ignore_ascii_case("STOP") => {
            "Google Gemini finished normally but returned no visible text.".to_string()
        }
        (Some(reason), _) => {
            format!("Google Gemini returned no visible text after finishing with {reason}.")
        }
        (None, _) => "Google Gemini returned no visible text.".to_string(),
    };

    if !details.is_empty() {
        message.push_str(" Metadata: ");
        message.push_str(&details.join("; "));
        message.push('.');
    }
    message
}

fn gemini_part_kinds(value: &Value) -> Vec<String> {
    let Some(parts) = value
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut kinds = Vec::new();
    for part in parts {
        let kind = if part.get("thought").and_then(Value::as_bool) == Some(true)
            && part.get("text").is_some()
        {
            "thought_text"
        } else if part.get("text").is_some() {
            "text"
        } else if part.get("functionCall").is_some() {
            "function_call"
        } else if part.get("functionResponse").is_some() {
            "function_response"
        } else if part.get("executableCode").is_some() {
            "executable_code"
        } else if part.get("codeExecutionResult").is_some() {
            "code_execution_result"
        } else if part.get("inlineData").is_some() || part.get("inline_data").is_some() {
            "inline_data"
        } else if part.get("fileData").is_some() || part.get("file_data").is_some() {
            "file_data"
        } else if part.get("thoughtSignature").is_some() {
            "thought_signature"
        } else {
            "unknown"
        };
        if !kinds.iter().any(|existing| existing == kind) {
            kinds.push(kind.to_string());
        }
    }
    kinds
}

fn gemini_safety_ratings(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|ratings| {
            ratings
                .iter()
                .filter_map(|rating| {
                    let category = rating.get("category").and_then(Value::as_str)?;
                    let probability = rating
                        .get("probability")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    Some(format!(
                        "{}:{}",
                        log_field(category),
                        log_field(probability)
                    ))
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .filter(|ratings| !ratings.is_empty())
}

fn log_field(value: &str) -> String {
    let cleaned = value
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if cleaned.is_empty() {
        "none".to_string()
    } else {
        cleaned
    }
}

fn gemini_body(request: &ProviderHttpRequest) -> Result<Value, InferenceError> {
    let mut contents = Vec::new();
    for message in &request.messages {
        let parts = message_to_parts(message);
        if !parts.is_empty() {
            let role = match message.role.trim().to_lowercase().as_str() {
                "assistant" | "model" => "model",
                _ => "user",
            };
            contents.push(json!({
                "role": role,
                "parts": parts
            }));
        }
    }

    if contents.is_empty() {
        return Err(InferenceError::invalid("Inference prompt cannot be empty."));
    }

    let mut body = json!({
        "contents": contents,
        "generationConfig": {
            "temperature": request.temperature.unwrap_or(0.2),
            "maxOutputTokens": request.max_tokens.unwrap_or(8192)
        }
    });
    if gemini_model_rejects_sampling_parameters(&request.model_id) {
        if let Some(generation_config) = body
            .get_mut("generationConfig")
            .and_then(Value::as_object_mut)
        {
            generation_config.remove("temperature");
        }
    }

    if let Some(system_prompt) = request.system_prompt.as_deref() {
        let system_prompt_trimmed = system_prompt.trim();
        if !system_prompt_trimmed.is_empty() {
            if let Some(obj) = body.as_object_mut() {
                obj.insert(
                    "systemInstruction".to_string(),
                    json!({
                        "parts": [{ "text": system_prompt_trimmed }]
                    }),
                );
            }
        }
    }

    if let Some(reasoning) = request
        .native_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(generation_config) = body
            .get_mut("generationConfig")
            .and_then(Value::as_object_mut)
        {
            let mut thinking_config = serde_json::Map::new();
            if gemini_model_prefers_thinking_level(&request.model_id) {
                if let Some(level) = gemini_thinking_level(reasoning) {
                    thinking_config.insert("thinkingLevel".to_string(), json!(level));
                }
            } else if gemini_reasoning_is_disabled(reasoning) {
                thinking_config.insert("thinkingBudget".to_string(), json!(0));
            } else if let Some(budget_tokens) = request.reasoning_budget_tokens {
                thinking_config.insert("thinkingBudget".to_string(), json!(budget_tokens));
            }

            if !thinking_config.is_empty() {
                generation_config
                    .insert("thinkingConfig".to_string(), Value::Object(thinking_config));
            }
        }
    }

    Ok(body)
}

fn gemini_model_rejects_sampling_parameters(model_id: &str) -> bool {
    matches!(
        model_id.trim().to_ascii_lowercase().as_str(),
        "gemini-3.6-flash" | "gemini-3.5-flash-lite"
    )
}

fn gemini_model_prefers_thinking_level(model_id: &str) -> bool {
    let normalized = model_id.trim().to_lowercase();
    normalized.starts_with("gemini-3") || normalized.contains("/gemini-3")
}

fn gemini_thinking_level(reasoning: &str) -> Option<&'static str> {
    match reasoning.trim().to_lowercase().as_str() {
        "off" | "none" | "disabled" | "false" | "0" | "minimal" | "min" => Some("MINIMAL"),
        "low" => Some("LOW"),
        "medium" => Some("MEDIUM"),
        "high" | "max" | "xhigh" | "x-high" | "ultra" | "extreme" => Some("HIGH"),
        _ => None,
    }
}

fn gemini_reasoning_is_disabled(reasoning: &str) -> bool {
    matches!(
        reasoning.trim().to_lowercase().as_str(),
        "off" | "none" | "disabled" | "false" | "0"
    )
}

fn message_to_parts(message: &InferenceMessage) -> Vec<Value> {
    let mut parts = Vec::new();
    let content = message.content.trim();
    if !content.is_empty() {
        parts.push(json!({ "text": content }));
    }
    for attachment in &message.attachments {
        if attachment.mime_type.starts_with("image/") {
            if let Some(data) = attachment
                .data_base64
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                parts.push(json!({
                    "inline_data": {
                        "mime_type": attachment.mime_type,
                        "data": data
                    }
                }));
            }
        } else if let Some(text) = attachment
            .text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(json!({ "text": super::grounding_contract::attachment_text_prompt(attachment, text) }));
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::ChatAttachment;

    #[test]
    fn test_gemini_payload_alternating_turns_and_system_instruction() {
        let payload = GeminiPayload;
        let client = BlockingClient::new();
        let request = ProviderHttpRequest {
            model_id: "gemini-3.5-flash".to_string(),
            system_prompt: Some("You are a helpful coding assistant.".to_string()),
            messages: vec![
                InferenceMessage {
                    role: "user".to_string(),
                    content: "Hello!".to_string(),
                    attachments: vec![],
                },
                InferenceMessage {
                    role: "assistant".to_string(),
                    content: "Hi there! How can I help you today?".to_string(),
                    attachments: vec![],
                },
                InferenceMessage {
                    role: "user".to_string(),
                    content: "Let's write some code.".to_string(),
                    attachments: vec![ChatAttachment {
                        name: "test.py".to_string(),
                        mime_type: "text/plain".to_string(),
                        byte_count: 14,
                        data_base64: None,
                        text: Some("print('hello')".to_string()),
                        approved_file_receipt: None,
                    }],
                },
            ],
            temperature: Some(0.7),
            max_tokens: Some(1024),
            native_reasoning: None,
            reasoning_budget_tokens: None,
            base_url: None,
        };

        let request_builder = payload
            .build_request(&client, "test-api-key", &request)
            .unwrap();
        let http_request = request_builder.build().unwrap();
        let body_bytes = http_request.body().unwrap().as_bytes().unwrap();
        let body: Value = serde_json::from_slice(body_bytes).unwrap();

        // Verify systemInstruction is mapped natively
        assert_eq!(
            body.pointer("/systemInstruction/parts/0/text").unwrap(),
            "You are a helpful coding assistant."
        );

        // Verify contents (multi-turn structure)
        let contents = body.get("contents").unwrap().as_array().unwrap();
        assert_eq!(contents.len(), 3);

        // Turn 0: User
        assert_eq!(contents[0].get("role").unwrap(), "user");
        assert_eq!(contents[0].pointer("/parts/0/text").unwrap(), "Hello!");

        // Turn 1: Model
        assert_eq!(contents[1].get("role").unwrap(), "model");
        assert_eq!(
            contents[1].pointer("/parts/0/text").unwrap(),
            "Hi there! How can I help you today?"
        );

        // Turn 2: User with attachment
        assert_eq!(contents[2].get("role").unwrap(), "user");
        assert_eq!(
            contents[2].pointer("/parts/0/text").unwrap(),
            "Let's write some code."
        );
        assert_eq!(
            contents[2].pointer("/parts/1/text").unwrap(),
            "Attached file test.py:\nprint('hello')"
        );

        // Verify generation config
        let temp = body
            .pointer("/generationConfig/temperature")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!((temp - 0.7).abs() < 1e-6);
        assert_eq!(
            body.pointer("/generationConfig/maxOutputTokens").unwrap(),
            1024
        );
    }

    #[test]
    fn gemini_follow_up_payload_includes_restored_public_search_grounding_text() {
        let grounding = concat!(
            "Local Web Search Context\n",
            "Query: ROC to SIN\n",
            "Engine: mod_declared_public_context\n\n",
            "Verified fare facts from the approved source."
        );
        let request = ProviderHttpRequest {
            model_id: "gemini-3.5-flash".to_string(),
            system_prompt: None,
            messages: vec![
                InferenceMessage {
                    role: "user".to_string(),
                    content: "/travel compare ROC to SIN".to_string(),
                    attachments: vec![ChatAttachment {
                        name: "local_web_search.md".to_string(),
                        mime_type: "text/markdown".to_string(),
                        byte_count: grounding.len(),
                        data_base64: None,
                        text: Some(grounding.to_string()),
                        approved_file_receipt: None,
                    }],
                },
                InferenceMessage {
                    role: "assistant".to_string(),
                    content: "I found the live options.".to_string(),
                    attachments: Vec::new(),
                },
                InferenceMessage {
                    role: "user".to_string(),
                    content: "Are you still working on that task?".to_string(),
                    attachments: Vec::new(),
                },
            ],
            temperature: None,
            max_tokens: None,
            native_reasoning: None,
            reasoning_budget_tokens: None,
            base_url: None,
        };

        let body = gemini_body(&request).expect("Gemini payload builds");
        assert_eq!(
            body.pointer("/contents/0/parts/1/text")
                .and_then(Value::as_str),
            Some(
                "Verified public-source evidence:\nQuery: ROC to SIN\nEngine: mod_declared_public_context\n\nVerified fare facts from the approved source."
            )
        );
        assert_eq!(
            body.pointer("/contents/2/parts/0/text")
                .and_then(Value::as_str),
            Some("Are you still working on that task?")
        );
    }

    #[test]
    fn gemini_payload_defaults_to_large_cloud_output_budget() {
        let request = ProviderHttpRequest {
            model_id: "gemini-3.5-flash".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Write a complete simulation.".to_string(),
                attachments: vec![],
            }],
            temperature: None,
            max_tokens: None,
            native_reasoning: None,
            reasoning_budget_tokens: None,
            base_url: None,
        };

        let body = gemini_body(&request).expect("gemini body builds");

        assert_eq!(
            body.pointer("/generationConfig/maxOutputTokens").unwrap(),
            8192
        );
    }

    #[test]
    fn latest_gemini_payload_omits_deprecated_sampling_parameters() {
        let request = ProviderHttpRequest {
            model_id: "gemini-3.6-flash".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Hello.".to_string(),
                attachments: vec![],
            }],
            temperature: Some(0.7),
            max_tokens: Some(4096),
            native_reasoning: Some("medium".to_string()),
            reasoning_budget_tokens: None,
            base_url: None,
        };

        let body = gemini_body(&request).expect("gemini body builds");

        assert!(body.pointer("/generationConfig/temperature").is_none());
        assert_eq!(
            body.pointer("/generationConfig/maxOutputTokens").unwrap(),
            4096
        );
    }

    #[test]
    fn gemini_payload_maps_gemini_three_native_reasoning_to_thinking_level() {
        let request = ProviderHttpRequest {
            model_id: "gemini-3.5-flash".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Use maximum reasoning.".to_string(),
                attachments: vec![],
            }],
            temperature: Some(0.2),
            max_tokens: Some(4096),
            native_reasoning: Some("xhigh".to_string()),
            reasoning_budget_tokens: Some(16_000),
            base_url: None,
        };

        let body = gemini_body(&request).expect("gemini body builds");

        assert_eq!(
            body.pointer("/generationConfig/thinkingConfig/thinkingLevel")
                .unwrap(),
            "HIGH"
        );
        assert!(body
            .pointer("/generationConfig/thinkingConfig/thinkingBudget")
            .is_none());
    }

    #[test]
    fn gemini_three_payload_maps_reasoning_off_to_minimal_thinking() {
        let request = ProviderHttpRequest {
            model_id: "gemini-3.5-flash".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Do not use extra reasoning.".to_string(),
                attachments: vec![],
            }],
            temperature: Some(0.2),
            max_tokens: Some(4096),
            native_reasoning: Some("off".to_string()),
            reasoning_budget_tokens: Some(16_000),
            base_url: None,
        };

        let body = gemini_body(&request).expect("gemini body builds");

        assert_eq!(
            body.pointer("/generationConfig/thinkingConfig/thinkingLevel")
                .unwrap(),
            "MINIMAL"
        );
        assert!(body
            .pointer("/generationConfig/thinkingConfig/thinkingBudget")
            .is_none());
    }

    #[test]
    fn gemini_payload_uses_budget_for_older_gemini_models() {
        let request = ProviderHttpRequest {
            model_id: "gemini-2.5-flash".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Use high reasoning.".to_string(),
                attachments: vec![],
            }],
            temperature: Some(0.2),
            max_tokens: Some(4096),
            native_reasoning: Some("high".to_string()),
            reasoning_budget_tokens: Some(8_000),
            base_url: None,
        };

        let body = gemini_body(&request).expect("gemini body builds");

        assert!(body
            .pointer("/generationConfig/thinkingConfig/thinkingLevel")
            .is_none());
        assert_eq!(
            body.pointer("/generationConfig/thinkingConfig/thinkingBudget")
                .unwrap(),
            8_000
        );
    }

    #[test]
    fn gemini_parse_response_combines_text_parts_and_finish_reason() {
        let payload = GeminiPayload;
        let parsed = payload
            .parse_response(json!({
                "candidates": [{
                    "finishReason": "STOP",
                    "content": {
                        "parts": [
                            { "text": "Hello, " },
                            { "text": "OOMU." }
                        ]
                    },
                    "safetyRatings": [{
                        "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                        "probability": "NEGLIGIBLE"
                    }]
                }]
            }))
            .expect("Gemini response should parse");

        assert_eq!(parsed.text, "Hello, OOMU.");
        assert_eq!(parsed.finish_reason.as_deref(), Some("STOP"));
    }

    #[test]
    fn gemini_parse_response_reports_prompt_block_for_empty_candidates() {
        let payload = GeminiPayload;
        let error = payload
            .parse_response(json!({
                "promptFeedback": {
                    "blockReason": "SAFETY",
                    "safetyRatings": [{
                        "category": "HARM_CATEGORY_HARASSMENT",
                        "probability": "HIGH"
                    }]
                },
                "candidates": []
            }))
            .expect_err("blocked prompt should not parse as usable text");

        assert_eq!(error.code, "provider_response_error");
        assert!(error
            .message
            .contains("blocked the prompt before returning visible text"));
        assert!(error.message.contains("promptBlockReason=SAFETY"));
        assert!(error
            .message
            .contains("promptSafety=HARM_CATEGORY_HARASSMENT:HIGH"));
    }

    #[test]
    fn gemini_parse_response_ignores_thought_text_as_visible_answer() {
        let payload = GeminiPayload;
        let error = payload
            .parse_response(json!({
                "candidates": [{
                    "finishReason": "STOP",
                    "content": {
                        "parts": [
                            { "thought": true, "text": "internal chain" }
                        ]
                    }
                }]
            }))
            .expect_err("thought text should not become the assistant answer");

        assert!(error
            .message
            .contains("finished normally but returned no visible text"));
        assert!(error.message.contains("partKinds=thought_text"));
    }

    #[test]
    fn gemini_stream_event_combines_text_parts_and_finish_reason() {
        let payload = GeminiPayload;
        let event = payload.parse_stream_event(&json!({
            "candidates": [{
                "finishReason": "MAX_TOKENS",
                "content": {
                    "parts": [
                        { "text": "partial " },
                        { "text": "answer" }
                    ]
                }
            }]
        }));

        assert_eq!(event.token.as_deref(), Some("partial answer"));
        assert_eq!(event.finish_reason.as_deref(), Some("MAX_TOKENS"));
        assert!(event.empty_response_message.is_none());
    }

    #[test]
    fn gemini_stream_event_reports_empty_max_tokens_diagnostic() {
        let payload = GeminiPayload;
        let event = payload.parse_stream_event(&json!({
            "candidates": [{
                "finishReason": "MAX_TOKENS",
                "content": {
                    "parts": [
                        { "thought": true, "text": "internal chain" },
                        { "thoughtSignature": "abc123" }
                    ]
                }
            }]
        }));

        assert!(event.token.is_none());
        assert_eq!(event.finish_reason.as_deref(), Some("MAX_TOKENS"));
        let message = event
            .empty_response_message
            .expect("empty terminal stream event should explain why");
        assert!(message.contains("reaching MAX_TOKENS"));
        assert!(message.contains("partKinds=thought_text,thought_signature"));
    }
}
