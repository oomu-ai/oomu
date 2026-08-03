use reqwest::Url;

pub(super) fn is_local_model_provider(provider_id: &str) -> bool {
    matches!(
        provider_id.trim().to_lowercase().replace('-', "_").as_str(),
        "local" | "local_model" | "local_gemma"
    )
}

pub(super) fn credential_aliases(provider_id: &str) -> &'static [&'static str] {
    match provider_id {
        "gemini" | "google" | "google_gemini" => &[
            "gemini",
            "google",
            "google_gemini",
            "gemini_pro",
            "gemini_flash",
        ],
        "openai" | "chatgpt" | "chat_gpt" => &["openai", "chatgpt", "chat_gpt"],
        "anthropic" | "claude" => &["anthropic", "claude"],
        "deepseek" | "deepseek_v3" | "deepseek_r1" => &["deepseek", "deepseek_v3", "deepseek_r1"],
        "qwen" => &["qwen"],
        "qwen_us" => &["qwen_us"],
        "zai" | "z_ai" => &["zai", "z_ai"],
        "zai_coding" => &["zai_coding"],
        "zhipu" => &["zhipu"],
        "moonshot" => &["moonshot"],
        "moonshot_global" => &["moonshot_global"],
        "custom" => &["custom"],
        "mistral" | "mistral_ai" => &["mistral", "mistral_ai"],
        "openrouter" => &["openrouter"],
        "synthetic" => &["synthetic"],
        "together" | "together_ai" => &["together", "together_ai"],
        "xai" | "x_ai" => &["xai", "x_ai"],
        _ => &[],
    }
}

pub(super) fn validate_provider_sync_origin(provider_id: &str, url: &Url) -> Result<(), String> {
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "Provider model synchronization URL has no host.".to_string())?;
    let allowed = match provider_id {
        "openai" | "chatgpt" | "chat_gpt" => host == "api.openai.com",
        "anthropic" | "claude" => host == "api.anthropic.com",
        "google" | "gemini" | "google_gemini" | "gemini_pro" | "gemini_flash" => {
            host == "generativelanguage.googleapis.com"
        }
        "xai" | "x_ai" => host == "api.x.ai",
        "deepseek" | "deepseek_v3" | "deepseek_r1" => host == "api.deepseek.com",
        "qwen" => host == "dashscope.aliyuncs.com",
        "qwen_us" => host == "dashscope-us.aliyuncs.com",
        "zai" | "z_ai" => host == "api.z.ai",
        "zai_coding" => host == "api.z.ai",
        "zhipu" => host == "open.bigmodel.cn",
        "moonshot" => host == "api.moonshot.cn",
        "moonshot_global" => host == "api.moonshot.ai",
        "mistral" | "mistral_ai" => host == "api.mistral.ai",
        "openrouter" => host == "openrouter.ai",
        "synthetic" => host == "api.synthetic.ai",
        "together" | "together_ai" => host == "api.together.xyz",
        _ => false,
    };
    allowed
        .then_some(())
        .ok_or_else(|| "Provider model synchronization origin is not allowlisted.".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TranslatedReasoningParameter {
    value: &'static str,
    budget_tokens: Option<u32>,
}

pub fn translate_reasoning_parameter(
    provider_id: &str,
    ui_setting: &str,
) -> (String, Option<usize>) {
    let translated = translate_reasoning_wire_parameter(provider_id, ui_setting);
    (
        translated.value.to_string(),
        translated.budget_tokens.map(|tokens| tokens as usize),
    )
}

fn translate_reasoning_wire_parameter(
    provider_id: &str,
    ui_setting: &str,
) -> TranslatedReasoningParameter {
    let provider_key = reasoning_capability_key(provider_id);
    let normalized = normalize_unified_reasoning_setting(ui_setting);
    if provider_key.contains("google") || provider_key.contains("gemini") {
        return translated_reasoning(normalized, "xhigh");
    }
    if provider_key.contains("openai")
        || provider_key.contains("chatgpt")
        || provider_key.starts_with("gpt")
        || provider_key.starts_with("o1")
        || provider_key.starts_with("o3")
        || provider_key.starts_with("o4")
        || provider_key.contains("x_ai")
        || provider_key.contains("deepseek")
        || provider_key.contains("qwen")
        || provider_key.contains("zai")
        || provider_key.contains("zhipu")
        || provider_key.contains("moonshot")
        || provider_key.contains("openrouter")
        || provider_key.contains("synthetic")
        || provider_key.contains("mistral")
    {
        return translated_reasoning(normalized, "max");
    }
    if provider_key.contains("anthropic") || provider_key.contains("claude") {
        return translated_reasoning(normalized, "max");
    }
    if normalized == "off" {
        reasoning_parameter("off", 0)
    } else {
        reasoning_parameter("on", 1)
    }
}

fn translated_reasoning(
    normalized: &'static str,
    maximum_value: &'static str,
) -> TranslatedReasoningParameter {
    match normalized {
        "off" => reasoning_parameter("off", 0),
        "low" => reasoning_parameter("low", 2_000),
        "medium" | "on" => reasoning_parameter("medium", 4_000),
        "high" => reasoning_parameter("high", 8_000),
        "max" => reasoning_parameter(maximum_value, 16_000),
        _ => reasoning_parameter("medium", 4_000),
    }
}

fn reasoning_parameter(value: &'static str, budget_tokens: u32) -> TranslatedReasoningParameter {
    TranslatedReasoningParameter {
        value,
        budget_tokens: Some(budget_tokens),
    }
}

fn normalize_unified_reasoning_setting(setting: &str) -> &'static str {
    match setting.trim().to_lowercase().as_str() {
        "off" => "off",
        "on" => "on",
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "max" | "xhigh" | "x-high" | "extreme" | "ultra" => "max",
        _ => "medium",
    }
}

fn reasoning_capability_key(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
}
