pub(super) fn fixed_provider_origin(provider_id: &str) -> Result<Option<&'static str>, String> {
    let origin = match provider_id {
        "openai" | "chatgpt" | "chat_gpt" => Some("https://api.openai.com"),
        "anthropic" | "claude" => Some("https://api.anthropic.com"),
        "google" | "gemini" | "google_gemini" | "gemini_pro" | "gemini_flash" => {
            Some("https://generativelanguage.googleapis.com")
        }
        "deepseek" | "deepseek_v3" | "deepseek_r1" => Some("https://api.deepseek.com"),
        "qwen" => Some("https://dashscope.aliyuncs.com"),
        "qwen_us" => Some("https://dashscope-us.aliyuncs.com"),
        "zai" | "z_ai" | "zai_coding" => Some("https://api.z.ai"),
        "zhipu" => Some("https://open.bigmodel.cn"),
        "moonshot" => Some("https://api.moonshot.cn"),
        "moonshot_global" => Some("https://api.moonshot.ai"),
        "mistral" | "mistral_ai" => Some("https://api.mistral.ai"),
        "openrouter" => Some("https://openrouter.ai"),
        "synthetic" => Some("https://api.synthetic.ai"),
        "together" | "together_ai" => Some("https://api.together.xyz"),
        "xai" | "x_ai" => Some("https://api.x.ai"),
        "custom" => None,
        _ => {
            return Err(
                "Provider identifier is not supported by native origin policy.".to_string(),
            );
        }
    };
    Ok(origin)
}
