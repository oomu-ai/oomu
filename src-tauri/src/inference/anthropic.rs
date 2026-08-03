use reqwest::blocking::{Client as BlockingClient, RequestBuilder as BlockingRequestBuilder};
use reqwest::{Client as AsyncClient, RequestBuilder as AsyncRequestBuilder};
use serde_json::{json, Value};

use super::{
    require_https_url, InferenceError, InferenceMessage, ProviderHttpRequest, ProviderPayload,
    ProviderResponse, ProviderStreamEvent,
};

const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicPayload;

impl ProviderPayload for AnthropicPayload {
    fn provider_name(&self) -> &'static str {
        "Anthropic"
    }

    fn build_request(
        &self,
        client: &BlockingClient,
        api_key: &str,
        request: &ProviderHttpRequest,
    ) -> Result<BlockingRequestBuilder, InferenceError> {
        require_https_url(ANTHROPIC_ENDPOINT)?;
        let body = anthropic_body(request)?;

        Ok(client
            .post(ANTHROPIC_ENDPOINT)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body))
    }

    fn build_stream_request(
        &self,
        client: &AsyncClient,
        api_key: &str,
        request: &ProviderHttpRequest,
    ) -> Result<AsyncRequestBuilder, InferenceError> {
        require_https_url(ANTHROPIC_ENDPOINT)?;
        let mut body = anthropic_body(request)?;
        body["stream"] = json!(true);

        Ok(client
            .post(ANTHROPIC_ENDPOINT)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body))
    }

    fn parse_response(&self, value: Value) -> Result<ProviderResponse, InferenceError> {
        let text = value
            .get("content")
            .and_then(Value::as_array)
            .and_then(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .find(|text| !text.trim().is_empty())
            })
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(InferenceError::provider(
                "Anthropic returned an empty response.",
            ));
        }

        Ok(ProviderResponse {
            text,
            response_id: value
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            finish_reason: value
                .get("stop_reason")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        })
    }

    fn parse_stream_event(&self, value: &Value) -> ProviderStreamEvent {
        ProviderStreamEvent {
            token: value
                .pointer("/delta/text")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            response_id: value
                .get("id")
                .or_else(|| value.pointer("/message/id"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            finish_reason: value
                .pointer("/delta/stop_reason")
                .or_else(|| value.pointer("/message/stop_reason"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            empty_response_message: None,
        }
    }
}

fn anthropic_body(request: &ProviderHttpRequest) -> Result<Value, InferenceError> {
    let messages = request
        .messages
        .iter()
        .filter_map(message_to_anthropic)
        .collect::<Vec<_>>();
    if messages.is_empty() {
        return Err(InferenceError::invalid("Inference prompt cannot be empty."));
    }

    let mut body = json!({
        "model": request.model_id,
        "messages": messages,
        "max_tokens": request.max_tokens.unwrap_or(8192),
        "temperature": request.temperature.unwrap_or(0.2)
    });
    if let Some(system_prompt) = request.system_prompt.as_deref() {
        if !system_prompt.trim().is_empty() {
            body["system"] = json!(system_prompt.trim());
        }
    }
    let claude_five = is_claude_five(&request.model_id);
    if claude_five {
        if let Some(object) = body.as_object_mut() {
            object.remove("temperature");
        }
    }
    if let Some(reasoning) = request
        .native_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if claude_five {
            apply_claude_five_reasoning(&mut body, &request.model_id, reasoning);
        } else if !anthropic_reasoning_is_disabled(reasoning) {
            apply_legacy_claude_thinking(&mut body, request, reasoning);
        }
    }
    Ok(body)
}

fn is_claude_five(model_id: &str) -> bool {
    let model = model_id.trim().to_ascii_lowercase();
    model.starts_with("claude-fable-5")
        || model.starts_with("claude-opus-5")
        || model.starts_with("claude-sonnet-5")
}

fn apply_claude_five_reasoning(body: &mut Value, model_id: &str, reasoning: &str) {
    if anthropic_reasoning_is_disabled(reasoning) {
        if !model_id
            .trim()
            .to_ascii_lowercase()
            .starts_with("claude-fable-5")
        {
            body["thinking"] = json!({ "type": "disabled" });
        }
        return;
    }
    body["output_config"] = json!({
        "effort": anthropic_effort(reasoning)
    });
}

fn apply_legacy_claude_thinking(body: &mut Value, request: &ProviderHttpRequest, reasoning: &str) {
    let budget_tokens = request
        .reasoning_budget_tokens
        .unwrap_or_else(|| match reasoning.trim().to_ascii_lowercase().as_str() {
            "low" => 2_000,
            "medium" => 4_000,
            "high" => 8_000,
            _ => 16_000,
        })
        .clamp(1_024, 128_000);
    let output_budget = request
        .max_tokens
        .unwrap_or(8_192)
        .max(budget_tokens.saturating_add(1_024));
    body["max_tokens"] = json!(output_budget);
    body["thinking"] = json!({
        "type": "enabled",
        "budget_tokens": budget_tokens
    });
    if let Some(object) = body.as_object_mut() {
        object.remove("temperature");
    }
}

fn anthropic_effort(reasoning: &str) -> &'static str {
    match reasoning.trim().to_ascii_lowercase().as_str() {
        "low" => "low",
        "medium" | "on" => "medium",
        "high" => "high",
        "max" | "xhigh" | "x-high" | "ultra" | "extreme" => "max",
        _ => "medium",
    }
}

fn anthropic_reasoning_is_disabled(reasoning: &str) -> bool {
    matches!(
        reasoning.trim().to_ascii_lowercase().as_str(),
        "off" | "none" | "disabled" | "false" | "0"
    )
}

fn message_to_anthropic(message: &InferenceMessage) -> Option<Value> {
    let content = message.content.trim();
    if message.role.eq_ignore_ascii_case("system") {
        return None;
    }
    let mut parts = Vec::new();
    if !content.is_empty() {
        parts.push(json!({ "type": "text", "text": content }));
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
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": attachment.mime_type,
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
            parts.push(json!({
                "type": "text",
                "text": super::grounding_contract::attachment_text_prompt(attachment, text)
            }));
        }
    }
    if parts.is_empty() {
        return None;
    }

    let content_value = if message.attachments.is_empty() {
        json!(content)
    } else {
        json!(parts)
    };

    Some(json!({
        "role": if message.role.eq_ignore_ascii_case("assistant") {
            "assistant"
        } else {
            "user"
        },
        "content": content_value
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_payload_defaults_to_large_cloud_output_budget() {
        let request = ProviderHttpRequest {
            model_id: "claude-sonnet-4-20250514".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Write a complete allocator design.".to_string(),
                attachments: vec![],
            }],
            temperature: None,
            max_tokens: None,
            native_reasoning: None,
            reasoning_budget_tokens: None,
            base_url: None,
        };

        let body = anthropic_body(&request).expect("anthropic body builds");

        assert_eq!(body.pointer("/max_tokens").unwrap(), 8192);
    }

    #[test]
    fn claude_five_payload_uses_adaptive_effort_without_legacy_budget() {
        let request = ProviderHttpRequest {
            model_id: "claude-fable-5".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Use default Claude reasoning.".to_string(),
                attachments: vec![],
            }],
            temperature: Some(0.2),
            max_tokens: Some(4096),
            native_reasoning: Some("high".to_string()),
            reasoning_budget_tokens: Some(4000),
            base_url: None,
        };

        let body = anthropic_body(&request).expect("anthropic body builds");

        assert_eq!(body.pointer("/output_config/effort").unwrap(), "high");
        assert!(body.get("thinking").is_none());
        assert!(body.get("temperature").is_none());
        assert_eq!(body.pointer("/max_tokens").unwrap(), 4096);
    }

    #[test]
    fn haiku_four_five_retains_supported_manual_thinking_budget() {
        let request = ProviderHttpRequest {
            model_id: "claude-haiku-4-5-20251001".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Think carefully.".to_string(),
                attachments: vec![],
            }],
            temperature: Some(0.2),
            max_tokens: Some(4096),
            native_reasoning: Some("high".to_string()),
            reasoning_budget_tokens: Some(4000),
            base_url: None,
        };

        let body = anthropic_body(&request).expect("anthropic body builds");

        assert_eq!(body.pointer("/thinking/type").unwrap(), "enabled");
        assert_eq!(body.pointer("/thinking/budget_tokens").unwrap(), 4000);
        assert_eq!(body.pointer("/max_tokens").unwrap(), 5024);
        assert!(body.get("temperature").is_none());
    }
}
