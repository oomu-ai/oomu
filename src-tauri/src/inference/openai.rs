use reqwest::blocking::{Client as BlockingClient, RequestBuilder as BlockingRequestBuilder};
use reqwest::{Client as AsyncClient, RequestBuilder as AsyncRequestBuilder};
use serde_json::{json, Value};

use super::{
    require_https_url, InferenceError, InferenceMessage, ProviderHttpRequest, ProviderPayload,
    ProviderResponse, ProviderStreamEvent,
};

const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const DEEPSEEK_ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
const QWEN_ENDPOINT: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";
const QWEN_US_ENDPOINT: &str =
    "https://dashscope-us.aliyuncs.com/compatible-mode/v1/chat/completions";
const ZAI_ENDPOINT: &str = "https://api.z.ai/api/paas/v4/chat/completions";
const ZAI_CODING_ENDPOINT: &str = "https://api.z.ai/api/coding/paas/v4/chat/completions";
const ZHIPU_ENDPOINT: &str = "https://open.bigmodel.cn/api/paas/v4/chat/completions";
const MOONSHOT_ENDPOINT: &str = "https://api.moonshot.cn/v1/chat/completions";
const MOONSHOT_GLOBAL_ENDPOINT: &str = "https://api.moonshot.ai/v1/chat/completions";
const MISTRAL_ENDPOINT: &str = "https://api.mistral.ai/v1/chat/completions";
const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
const SYNTHETIC_ENDPOINT: &str = "https://api.synthetic.ai/v1/chat/completions";
const TOGETHER_ENDPOINT: &str = "https://api.together.xyz/v1/chat/completions";
const XAI_ENDPOINT: &str = "https://api.x.ai/v1/chat/completions";

pub struct OpenAiPayload {
    endpoint: Option<&'static str>,
    provider_name: &'static str,
    reasoning_protocol: ReasoningProtocol,
}

#[derive(Clone, Copy)]
enum ReasoningProtocol {
    None,
    Effort {
        disabled_value: Option<&'static str>,
    },
    UnifiedGateway,
    DeepSeekThinking,
    ThinkingToggle,
    QwenThinking,
    MoonshotThinking,
    XaiEffort,
}

impl OpenAiPayload {
    pub fn openai() -> Self {
        Self {
            endpoint: Some(OPENAI_ENDPOINT),
            provider_name: "OpenAI",
            reasoning_protocol: ReasoningProtocol::Effort {
                disabled_value: None,
            },
        }
    }

    pub fn deepseek() -> Self {
        Self {
            endpoint: Some(DEEPSEEK_ENDPOINT),
            provider_name: "DeepSeek",
            reasoning_protocol: ReasoningProtocol::DeepSeekThinking,
        }
    }

    pub fn qwen(provider_id: &str) -> Self {
        let (endpoint, provider_name) = if provider_id == "qwen_us" {
            (QWEN_US_ENDPOINT, "Alibaba Qwen US")
        } else {
            (QWEN_ENDPOINT, "Alibaba Qwen")
        };
        Self {
            endpoint: Some(endpoint),
            provider_name,
            reasoning_protocol: ReasoningProtocol::QwenThinking,
        }
    }

    pub fn zhipu() -> Self {
        Self {
            endpoint: Some(ZHIPU_ENDPOINT),
            provider_name: "Zhipu AI",
            reasoning_protocol: ReasoningProtocol::ThinkingToggle,
        }
    }

    pub fn zai() -> Self {
        Self {
            endpoint: Some(ZAI_ENDPOINT),
            provider_name: "Z.AI",
            reasoning_protocol: ReasoningProtocol::ThinkingToggle,
        }
    }

    pub fn zai_coding() -> Self {
        Self {
            endpoint: Some(ZAI_CODING_ENDPOINT),
            provider_name: "Z.AI Coding Plan",
            reasoning_protocol: ReasoningProtocol::ThinkingToggle,
        }
    }

    pub fn moonshot(provider_id: &str) -> Self {
        let (endpoint, provider_name) = if provider_id == "moonshot_global" {
            (MOONSHOT_GLOBAL_ENDPOINT, "Moonshot AI Global")
        } else {
            (MOONSHOT_ENDPOINT, "Moonshot AI")
        };
        Self {
            endpoint: Some(endpoint),
            provider_name,
            reasoning_protocol: ReasoningProtocol::MoonshotThinking,
        }
    }

    pub fn custom() -> Self {
        Self {
            endpoint: None,
            provider_name: "Custom Provider",
            reasoning_protocol: ReasoningProtocol::None,
        }
    }

    pub fn mistral() -> Self {
        Self {
            endpoint: Some(MISTRAL_ENDPOINT),
            provider_name: "Mistral AI",
            reasoning_protocol: ReasoningProtocol::Effort {
                disabled_value: None,
            },
        }
    }

    pub fn openrouter() -> Self {
        Self {
            endpoint: Some(OPENROUTER_ENDPOINT),
            provider_name: "OpenRouter",
            reasoning_protocol: ReasoningProtocol::UnifiedGateway,
        }
    }

    pub fn synthetic() -> Self {
        Self {
            endpoint: Some(SYNTHETIC_ENDPOINT),
            provider_name: "Synthetic",
            reasoning_protocol: ReasoningProtocol::Effort {
                disabled_value: Some("off"),
            },
        }
    }

    pub fn together() -> Self {
        Self {
            endpoint: Some(TOGETHER_ENDPOINT),
            provider_name: "Together AI",
            reasoning_protocol: ReasoningProtocol::None,
        }
    }

    pub fn xai() -> Self {
        Self {
            endpoint: Some(XAI_ENDPOINT),
            provider_name: "xAI",
            reasoning_protocol: ReasoningProtocol::XaiEffort,
        }
    }

    fn endpoint_url(&self, request: &ProviderHttpRequest) -> Result<String, InferenceError> {
        let endpoint = match self.endpoint {
            // Compiled provider endpoints are authoritative. A renderer-saved
            // base URL must never retarget a known provider's Keychain secret.
            Some(endpoint) => endpoint,
            None => request.base_url.as_deref().ok_or_else(|| {
                InferenceError::invalid(format!(
                    "{} requires a configured base URL.",
                    self.provider_name
                ))
            })?,
        };
        let url = chat_completions_url(endpoint)?;
        require_https_url(&url)?;
        Ok(url)
    }
}

impl ProviderPayload for OpenAiPayload {
    fn provider_name(&self) -> &'static str {
        self.provider_name
    }

    fn build_request(
        &self,
        client: &BlockingClient,
        api_key: &str,
        request: &ProviderHttpRequest,
    ) -> Result<BlockingRequestBuilder, InferenceError> {
        let body = openai_body(request, self.reasoning_protocol, self.provider_name)?;
        let url = self.endpoint_url(request)?;

        Ok(client.post(url).bearer_auth(api_key).json(&body))
    }

    fn build_stream_request(
        &self,
        client: &AsyncClient,
        api_key: &str,
        request: &ProviderHttpRequest,
    ) -> Result<AsyncRequestBuilder, InferenceError> {
        let mut body = openai_body(request, self.reasoning_protocol, self.provider_name)?;
        body["stream"] = json!(true);
        let url = self.endpoint_url(request)?;

        Ok(client.post(url).bearer_auth(api_key).json(&body))
    }

    fn parse_response(&self, value: Value) -> Result<ProviderResponse, InferenceError> {
        let text = value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            if self.provider_name == "DeepSeek" && deepseek_reasoning_observed(&value) {
                return Err(InferenceError::deepseek_reasoning_without_answer());
            }
            return Err(InferenceError::provider(format!(
                "{} returned an empty response.",
                self.provider_name
            )));
        }

        Ok(ProviderResponse {
            text,
            response_id: value
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            finish_reason: value
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        })
    }

    fn parse_stream_event(&self, value: &Value) -> ProviderStreamEvent {
        ProviderStreamEvent {
            token: value
                .pointer("/choices/0/delta/content")
                .or_else(|| value.pointer("/choices/0/message/content"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            reasoning_observed: self.provider_name == "DeepSeek"
                && deepseek_reasoning_observed(value),
            response_id: value
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            finish_reason: value
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            empty_response_message: None,
        }
    }
}

fn deepseek_reasoning_observed(value: &Value) -> bool {
    value
        .pointer("/choices/0/delta/reasoning_content")
        .or_else(|| value.pointer("/choices/0/message/reasoning_content"))
        .and_then(Value::as_str)
        .is_some_and(|reasoning| !reasoning.trim().is_empty())
}

fn openai_body(
    request: &ProviderHttpRequest,
    reasoning_protocol: ReasoningProtocol,
    provider_name: &str,
) -> Result<Value, InferenceError> {
    let mut messages = Vec::new();
    if let Some(system_prompt) = request.system_prompt.as_deref() {
        if !system_prompt.trim().is_empty() {
            messages.push(json!({
                "role": "system",
                "content": system_prompt.trim()
            }));
        }
    }
    messages.extend(request.messages.iter().filter_map(message_to_openai));
    if messages.is_empty() {
        return Err(InferenceError::invalid("Inference prompt cannot be empty."));
    }

    let mut body = json!({
        "model": provider_model_id(&request.model_id, provider_name),
        "messages": messages,
        "temperature": request.temperature.unwrap_or(0.2),
    });
    let output_limit_key = if (provider_name == "OpenAI"
        && is_openai_reasoning_model(&request.model_id))
        || provider_name == "Moonshot AI Global"
    {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    body[output_limit_key] = json!(request.max_tokens.unwrap_or(4096));

    apply_reasoning_protocol(&mut body, request, reasoning_protocol, provider_name);

    Ok(body)
}

fn apply_reasoning_protocol(
    body: &mut Value,
    request: &ProviderHttpRequest,
    protocol: ReasoningProtocol,
    provider_name: &str,
) {
    let Some(reasoning) = request
        .native_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let disabled = reasoning_is_disabled(reasoning);
    match protocol {
        ReasoningProtocol::None => {}
        ReasoningProtocol::Effort { disabled_value } => {
            if disabled {
                if let Some(value) = disabled_value {
                    body["reasoning_effort"] = json!(value);
                }
            } else {
                body["reasoning_effort"] =
                    json!(provider_effort(provider_name, &request.model_id, reasoning,));
                if provider_name == "OpenAI" && is_openai_reasoning_model(&request.model_id) {
                    remove_temperature(body);
                }
            }
        }
        ReasoningProtocol::UnifiedGateway => {
            body["reasoning"] = json!({
                "effort": if disabled { "none" } else { normalized_effort(reasoning) }
            });
        }
        ReasoningProtocol::DeepSeekThinking => {
            body["thinking"] = json!({
                "type": if disabled { "disabled" } else { "enabled" }
            });
            if !disabled {
                body["reasoning_effort"] = json!(if normalized_effort(reasoning) == "max" {
                    "max"
                } else {
                    "high"
                });
                remove_temperature(body);
            }
        }
        ReasoningProtocol::ThinkingToggle => {
            body["thinking"] = json!({
                "type": if disabled { "disabled" } else { "enabled" }
            });
            if !disabled && is_glm_5_2(&request.model_id) {
                body["reasoning_effort"] = json!(normalized_effort(reasoning));
            }
        }
        ReasoningProtocol::QwenThinking => {
            apply_qwen_reasoning(body, request, provider_name, disabled);
        }
        ReasoningProtocol::MoonshotThinking => {
            apply_moonshot_reasoning(body, &request.model_id, reasoning, disabled);
        }
        ReasoningProtocol::XaiEffort => apply_xai_reasoning(body, &request.model_id, reasoning),
    }
}

fn apply_qwen_reasoning(
    body: &mut Value,
    request: &ProviderHttpRequest,
    provider_name: &str,
    disabled: bool,
) {
    if provider_model_id(&request.model_id, provider_name).contains("coder") {
        return;
    }
    body["enable_thinking"] = json!(!disabled);
    if !disabled {
        if let Some(tokens) = request.reasoning_budget_tokens.filter(|tokens| *tokens > 0) {
            body["thinking_budget"] = json!(tokens);
        }
    }
}

fn apply_moonshot_reasoning(body: &mut Value, model_id: &str, reasoning: &str, disabled: bool) {
    if model_id.trim().contains("k2.7-code") {
        return;
    }
    if model_id.trim().eq_ignore_ascii_case("kimi-k3") {
        if !disabled {
            body["thinking"] = json!({ "type": "enabled" });
            body["reasoning_effort"] = json!(moonshot_effort(reasoning));
        }
        return;
    }
    body["thinking"] = json!({
        "type": if disabled { "disabled" } else { "enabled" }
    });
}

fn provider_model_id<'a>(model_id: &'a str, provider_name: &str) -> &'a str {
    let model_id = model_id.trim();
    let prefix = match provider_name {
        "Alibaba Qwen" | "Alibaba Qwen US" => "dashscope/",
        "Z.AI" | "Z.AI Coding Plan" | "Zhipu AI" => "zai/",
        "xAI" => "xai/",
        _ => return model_id,
    };
    model_id.strip_prefix(prefix).unwrap_or(model_id)
}

fn is_openai_reasoning_model(model_id: &str) -> bool {
    let model = model_id.trim().to_ascii_lowercase();
    model.starts_with("gpt-5") || model.starts_with('o')
}

fn remove_temperature(body: &mut Value) {
    if let Some(object) = body.as_object_mut() {
        object.remove("temperature");
    }
}

fn apply_xai_reasoning(body: &mut Value, model_id: &str, reasoning: &str) {
    let model = provider_model_id(model_id, "xAI").to_ascii_lowercase();
    if model.contains("non-reasoning") || model.starts_with("grok-build") {
        return;
    }
    let disabled = reasoning_is_disabled(reasoning);
    if model == "grok-4.3" {
        body["reasoning_effort"] = json!(if disabled {
            "none"
        } else {
            clamp_effort(reasoning, "high")
        });
    } else if model.starts_with("grok-4.20-multi-agent") {
        if !disabled {
            body["reasoning_effort"] = json!(clamp_effort(reasoning, "xhigh"));
        }
    } else if model == "grok-4.5" {
        if !disabled {
            body["reasoning_effort"] = json!(clamp_effort(reasoning, "high"));
        }
    }
}

fn clamp_effort(reasoning: &str, maximum: &'static str) -> &'static str {
    match normalized_effort(reasoning) {
        "minimal" | "low" => "low",
        "medium" => "medium",
        "high" => "high",
        _ => maximum,
    }
}

fn moonshot_effort(reasoning: &str) -> &'static str {
    match normalized_effort(reasoning) {
        "minimal" | "low" => "low",
        "medium" | "high" => "high",
        _ => "max",
    }
}

fn is_glm_5_2(model_id: &str) -> bool {
    provider_model_id(model_id, "Z.AI").eq_ignore_ascii_case("glm-5.2")
}

fn provider_effort(provider_name: &str, model_id: &str, reasoning: &str) -> &'static str {
    let normalized = normalized_effort(reasoning);
    if normalized != "max" {
        return normalized;
    }
    let model = model_id.trim().to_ascii_lowercase();
    match provider_name {
        "OpenAI" if model.starts_with("gpt-5.6") => "max",
        "OpenAI" if model == "gpt-5-mini" || model == "gpt-5-nano" => "high",
        "OpenAI" => "xhigh",
        "DeepSeek" => "max",
        "Zhipu AI" => "xhigh",
        "Synthetic" if model.contains("gpt-5.6-sol") => "ultra",
        "Synthetic" => "xhigh",
        _ => "high",
    }
}

fn normalized_effort(reasoning: &str) -> &'static str {
    match reasoning.trim().to_ascii_lowercase().as_str() {
        "on" => "medium",
        "minimal" | "min" => "minimal",
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "max" | "xhigh" | "x-high" | "ultra" | "extreme" => "max",
        _ => "medium",
    }
}

fn reasoning_is_disabled(reasoning: &str) -> bool {
    matches!(
        reasoning.trim().to_ascii_lowercase().as_str(),
        "off" | "none" | "disabled" | "false" | "0"
    )
}

fn message_to_openai(message: &InferenceMessage) -> Option<Value> {
    let content = message.content.trim();
    let has_attachments = message.attachments.iter().any(|attachment| {
        attachment.mime_type.starts_with("image/")
            && attachment
                .data_base64
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
            || attachment
                .text
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
    });
    if content.is_empty() && !has_attachments {
        return None;
    }

    if has_attachments {
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
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:{};base64,{}", attachment.mime_type, data)
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
        return Some(json!({
            "role": normalize_chat_role(&message.role),
            "content": parts
        }));
    }

    Some(json!({
        "role": normalize_chat_role(&message.role),
        "content": content
    }))
}

fn normalize_chat_role(role: &str) -> &'static str {
    match role.trim().to_lowercase().as_str() {
        "assistant" => "assistant",
        "system" => "system",
        _ => "user",
    }
}

fn chat_completions_url(endpoint: &str) -> Result<String, InferenceError> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(InferenceError::invalid(
            "Provider base URL cannot be empty for chat inference.",
        ));
    }
    if endpoint.ends_with("/chat/completions") {
        Ok(endpoint.to_string())
    } else if endpoint.ends_with('/') {
        Ok(format!("{endpoint}chat/completions"))
    } else {
        Ok(format!("{endpoint}/chat/completions"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_completions_url_accepts_full_endpoint() {
        assert_eq!(
            chat_completions_url("https://openrouter.ai/api/v1/chat/completions").unwrap(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }

    #[test]
    fn chat_completions_url_appends_suffix_to_base_url() {
        assert_eq!(
            chat_completions_url("https://openrouter.ai/api/v1").unwrap(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://openrouter.ai/api/v1/").unwrap(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }

    #[test]
    fn known_provider_endpoint_cannot_be_overridden_by_renderer_configuration() {
        let request = ProviderHttpRequest {
            model_id: "gpt-5.5".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
                attachments: vec![],
            }],
            temperature: None,
            max_tokens: None,
            native_reasoning: None,
            reasoning_budget_tokens: None,
            base_url: Some("https://credential-sink.example/v1".to_string()),
        };

        assert_eq!(
            OpenAiPayload::openai().endpoint_url(&request).unwrap(),
            OPENAI_ENDPOINT
        );
        assert_eq!(
            OpenAiPayload::custom().endpoint_url(&request).unwrap(),
            "https://credential-sink.example/v1/chat/completions"
        );
    }

    #[test]
    fn canonical_2026_openai_compatible_providers_use_their_fixed_endpoints() {
        let request = ProviderHttpRequest {
            model_id: "catalog-model".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
                attachments: vec![],
            }],
            temperature: None,
            max_tokens: None,
            native_reasoning: None,
            reasoning_budget_tokens: None,
            base_url: Some("https://credential-sink.example/v1".to_string()),
        };

        for (provider, endpoint) in [
            (OpenAiPayload::qwen("qwen"), QWEN_ENDPOINT),
            (OpenAiPayload::qwen("qwen_us"), QWEN_US_ENDPOINT),
            (OpenAiPayload::zai(), ZAI_ENDPOINT),
            (OpenAiPayload::zai_coding(), ZAI_CODING_ENDPOINT),
            (OpenAiPayload::zhipu(), ZHIPU_ENDPOINT),
            (OpenAiPayload::moonshot("moonshot"), MOONSHOT_ENDPOINT),
            (
                OpenAiPayload::moonshot("moonshot_global"),
                MOONSHOT_GLOBAL_ENDPOINT,
            ),
            (OpenAiPayload::synthetic(), SYNTHETIC_ENDPOINT),
            (OpenAiPayload::xai(), XAI_ENDPOINT),
        ] {
            assert_eq!(provider.endpoint_url(&request).unwrap(), endpoint);
        }
    }

    #[test]
    fn openai_compatible_payload_defaults_to_expanded_output_budget() {
        let request = ProviderHttpRequest {
            model_id: "gpt-4.1".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Write a complete implementation.".to_string(),
                attachments: vec![],
            }],
            temperature: None,
            max_tokens: None,
            native_reasoning: None,
            reasoning_budget_tokens: None,
            base_url: None,
        };

        let body = openai_body(
            &request,
            ReasoningProtocol::Effort {
                disabled_value: None,
            },
            "OpenAI",
        )
        .expect("openai body builds");

        assert_eq!(body.pointer("/max_tokens").unwrap(), 4096);
    }

    #[test]
    fn openai_payload_maps_native_reasoning_effort() {
        let request = ProviderHttpRequest {
            model_id: "gpt-5.5".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Think carefully.".to_string(),
                attachments: vec![],
            }],
            temperature: Some(0.2),
            max_tokens: Some(2048),
            native_reasoning: Some("high".to_string()),
            reasoning_budget_tokens: Some(8000),
            base_url: None,
        };

        let body = openai_body(
            &request,
            ReasoningProtocol::Effort {
                disabled_value: None,
            },
            "OpenAI",
        )
        .expect("openai body builds");

        assert_eq!(body.pointer("/reasoning_effort").unwrap(), "high");
        assert_eq!(body.pointer("/max_completion_tokens").unwrap(), 2048);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn provider_reasoning_controls_are_written_in_native_wire_formats() {
        let mut request = ProviderHttpRequest {
            model_id: "deepseek-v4-pro".to_string(),
            system_prompt: None,
            messages: vec![InferenceMessage {
                role: "user".to_string(),
                content: "Think carefully.".to_string(),
                attachments: vec![],
            }],
            temperature: Some(0.2),
            max_tokens: Some(4096),
            native_reasoning: Some("high".to_string()),
            reasoning_budget_tokens: Some(8000),
            base_url: None,
        };

        let deepseek =
            openai_body(&request, ReasoningProtocol::DeepSeekThinking, "DeepSeek").unwrap();
        assert_eq!(deepseek.pointer("/thinking/type").unwrap(), "enabled");
        assert_eq!(deepseek.pointer("/reasoning_effort").unwrap(), "high");
        assert!(deepseek.get("temperature").is_none());

        let qwen = openai_body(&request, ReasoningProtocol::QwenThinking, "Alibaba Qwen").unwrap();
        assert_eq!(qwen.pointer("/enable_thinking").unwrap(), true);
        assert_eq!(qwen.pointer("/thinking_budget").unwrap(), 8000);

        request.model_id = "grok-4.5".to_string();
        request.native_reasoning = Some("max".to_string());
        let grok = openai_body(&request, ReasoningProtocol::XaiEffort, "xAI").unwrap();
        assert_eq!(grok.pointer("/reasoning_effort").unwrap(), "high");

        let openrouter =
            openai_body(&request, ReasoningProtocol::UnifiedGateway, "OpenRouter").unwrap();
        assert_eq!(openrouter.pointer("/reasoning/effort").unwrap(), "max");

        request.model_id = "zai/glm-5.2".to_string();
        request.native_reasoning = Some("max".to_string());
        let glm = openai_body(&request, ReasoningProtocol::ThinkingToggle, "Z.AI").unwrap();
        assert_eq!(glm.pointer("/model").unwrap(), "glm-5.2");
        assert_eq!(glm.pointer("/thinking/type").unwrap(), "enabled");
        assert_eq!(glm.pointer("/reasoning_effort").unwrap(), "max");

        request.native_reasoning = Some("off".to_string());
        let disabled =
            openai_body(&request, ReasoningProtocol::DeepSeekThinking, "DeepSeek").unwrap();
        assert_eq!(disabled.pointer("/thinking/type").unwrap(), "disabled");

        for (provider, input, expected) in [
            ("Alibaba Qwen", "dashscope/qwen3.5-plus", "qwen3.5-plus"),
            ("Zhipu AI", "zai/glm-5", "glm-5"),
            ("xAI", "xai/grok-4.20", "grok-4.20"),
        ] {
            request.model_id = input.to_string();
            let body = openai_body(&request, ReasoningProtocol::None, provider).unwrap();
            assert_eq!(body.pointer("/model").unwrap(), expected);
        }
    }

    #[test]
    fn deepseek_reasoning_is_detected_without_exposing_hidden_reasoning_as_text() {
        let provider = OpenAiPayload::deepseek();
        let event = provider.parse_stream_event(&json!({
            "choices": [{
                "delta": { "reasoning_content": "private reasoning" },
                "finish_reason": null
            }]
        }));

        assert!(event.reasoning_observed);
        assert_eq!(event.token, None);

        let error = provider
            .parse_response(json!({
                "choices": [{
                    "message": {
                        "reasoning_content": "private reasoning",
                        "content": ""
                    },
                    "finish_reason": "length"
                }]
            }))
            .unwrap_err();
        assert_eq!(error.code, "deepseek_reasoning_without_answer");
    }

    #[test]
    fn deepseek_visible_answer_remains_authoritative_when_reasoning_is_present() {
        let provider = OpenAiPayload::deepseek();
        let response = provider
            .parse_response(json!({
                "choices": [{
                    "message": {
                        "reasoning_content": "private reasoning",
                        "content": "Hello from DeepSeek."
                    },
                    "finish_reason": "stop"
                }]
            }))
            .unwrap();

        assert_eq!(response.text, "Hello from DeepSeek.");
    }
}
