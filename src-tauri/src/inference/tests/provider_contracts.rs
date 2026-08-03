use super::*;

#[test]
fn provider_payload_accepts_configured_route_aliases() {
    for provider_id in [
        "chat_gpt",
        "gemini_pro",
        "gemini_flash",
        "deepseek_v3",
        "deepseek_r1",
        "qwen",
        "qwen_us",
        "zai",
        "zai_coding",
        "zhipu",
        "moonshot",
        "moonshot_global",
        "mistral",
        "openrouter",
        "synthetic",
        "together",
        "xai",
        "custom",
    ] {
        assert!(
            payload_for_provider(provider_id).is_ok(),
            "provider route alias should be supported: {provider_id}"
        );
    }
}

#[test]
fn provider_request_normalization_does_not_invent_external_tool_capabilities() {
    let normalized = normalize_request(InferenceRequest {
        provider_id: "google".to_string(),
        model_id: "gemini-3.5-flash".to_string(),
        system_prompt: Some(" Be useful. ".to_string()),
        messages: vec![test_message("User", "Write a local test file.")],
        prompt: None,
        temperature: None,
        max_tokens: None,
        reasoning: None,
        reasoning_budget_tokens: None,
        base_url: None,
        api_key_label: None,
        api_key: None,
    })
    .expect("external provider request normalizes");
    let system_prompt = normalized
        .system_prompt
        .as_deref()
        .expect("system prompt is retained");

    assert_eq!(system_prompt, "Be useful.");
    assert!(!system_prompt.contains("file_write"));
    assert!(!system_prompt.contains("terminal_execute"));
}

#[test]
fn credential_aliases_accept_configured_route_aliases() {
    assert_eq!(
        credential_aliases("chat_gpt"),
        ["openai", "chatgpt", "chat_gpt"]
    );
    assert_eq!(
        credential_aliases("deepseek_r1"),
        ["deepseek", "deepseek_v3", "deepseek_r1"]
    );
    assert_eq!(credential_aliases("openrouter"), ["openrouter"]);
    assert_eq!(credential_aliases("qwen"), ["qwen"]);
    assert_eq!(credential_aliases("qwen_us"), ["qwen_us"]);
    assert_eq!(credential_aliases("zai"), ["zai", "z_ai"]);
    assert_eq!(credential_aliases("zai_coding"), ["zai_coding"]);
    assert_eq!(credential_aliases("zhipu"), ["zhipu"]);
    assert_eq!(credential_aliases("moonshot"), ["moonshot"]);
    assert_eq!(credential_aliases("moonshot_global"), ["moonshot_global"]);
    assert_eq!(
        credential_aliases("together_ai"),
        ["together", "together_ai"]
    );
}

#[test]
fn sprint_59_configured_api_key_is_preferred_over_labels() {
    let key = load_provider_api_key("openai", Some("OPENAI_API_KEY"), Some(" sk-test-db "))
        .expect("configured key should resolve");

    assert_eq!(key, "sk-test-db");
}

#[test]
fn sprint_59_masked_configured_api_key_is_ignored() {
    let error = load_provider_api_key(
        "unit_test_provider",
        Some("UNIT_TEST_PROVIDER_KEY_THAT_SHOULD_NOT_EXIST"),
        Some("••••••"),
    )
    .expect_err("masked key should not resolve");

    assert_eq!(error.code, "credential_unavailable");
}

#[test]
fn persisted_custom_route_cannot_resolve_renderer_selected_environment_secret_label() {
    const LABEL: &str = "OOMU_PROVIDER_ENV_EXFIL_REGRESSION_CANARY";
    const SECRET: &str = "environment-secret-must-not-be-routed";
    std::env::set_var(LABEL, SECRET);
    let (manager, db_path) = temporary_agent_manager("provider-env-label-boundary");
    let mut provider = configured_provider("prov-env-boundary", "custom", "custom-model");
    provider.api_key_label = LABEL.to_string();
    provider.api_key = None;
    manager.upsert_provider_config(provider).unwrap();

    // The legacy generic loader can resolve explicit process labels for
    // non-persisted compatibility routes; persisted routes must stop before
    // that loader and require their origin-bound Keychain value instead.
    assert_eq!(
        load_provider_api_key("custom", Some(LABEL), None).unwrap(),
        SECRET
    );
    let error = resolve_provider_route(&manager, "prov-env-boundary")
        .expect_err("persisted route without a Keychain secret must fail closed");
    assert_eq!(error.code, "credential_unavailable");
    assert!(error.message.contains("Keychain credential"));

    std::env::remove_var(LABEL);
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn sprint_304_local_provider_route_never_hydrates_a_keychain_secret() {
    let (manager, db_path) = temporary_agent_manager("local-route-no-keychain");
    let mut provider = configured_provider(
        "prov-local-sprint-304",
        "local_model",
        "gemma-4-E4B-it-qat-q4_0-gguf",
    );
    provider.auth_method = "custom".to_string();
    provider.base_url.clear();
    manager.upsert_provider_config(provider).unwrap();
    assert!(is_local_model_provider("local_model"));

    let route = resolve_provider_route(&manager, "prov-local-sprint-304")
        .expect("a local provider route must resolve without reading Keychain");
    assert_eq!(route.catalog_provider_id, "local_model");
    assert!(route.overrides.api_key.is_none());

    let _ = std::fs::remove_file(db_path);
}

#[test]
fn sprint_59_require_https_url_blocks_plaintext_provider_urls() {
    assert!(require_https_url("https://api.openai.com/v1/chat/completions").is_ok());

    let error = require_https_url("http://example.com/v1/chat/completions")
        .expect_err("http provider URLs must be rejected");
    assert_eq!(error.code, "invalid_request");
}

#[test]
fn provider_http_errors_are_redacted_and_rate_limited() {
    let url = Url::parse(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini:streamGenerateContent?key=secret-value&alt=sse",
        )
        .unwrap();
    let message = provider_http_status_message(StatusCode::BAD_REQUEST, Some(&url));
    assert!(message.contains("400 Bad Request"));
    assert!(message.contains(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini:streamGenerateContent"
    ));
    assert!(!message.contains("secret-value"));
    assert!(!message.contains("?key="));

    let network = InferenceError::network(
        "HTTP status client error for url (https://example.test/path?key=secret-value&alt=sse)",
    );
    assert_eq!(network.code, "provider_network_error");
    assert!(!network.message.contains("secret-value"));
    assert!(network.message.contains("key=[redacted]"));

    let rate_limited = InferenceError::provider_rate_limited();
    assert_eq!(rate_limited.code, "provider_rate_limited");
    assert_eq!(rate_limited.boundary, "provider_api");
    assert!(rate_limited.message.contains("HTTP 429"));
}

#[test]
fn chat_response_claim_errors_are_stable_and_user_safe() {
    let conflict = chat_turn_response_claim_error(rusqlite::Error::InvalidParameterName(
        "chat_turn_response_claim_conflict".to_string(),
    ));
    assert_eq!(conflict.code, "chat_turn_already_running");
    assert_eq!(
        conflict.message,
        "OOMU is already working on this message. Reply pending."
    );
    assert!(!conflict.message.contains("chat_turns"));

    let database_failure = chat_turn_response_claim_error(rusqlite::Error::InvalidParameterName(
        "UNIQUE constraint failed: chat_turns.generation_token".to_string(),
    ));
    assert_eq!(database_failure.code, "chat_turn_persistence_failed");
    assert_eq!(
        database_failure.message,
        "OOMU could not reserve this response. Try again."
    );
    assert!(!database_failure.message.contains("UNIQUE"));
}

#[test]
fn provider_model_sync_origins_are_fixed_per_provider() {
    for (provider_id, endpoint) in [
        ("openai", "https://api.openai.com/v1/models"),
        ("chatgpt", "https://api.openai.com/v1/models"),
        ("anthropic", "https://api.anthropic.com/v1/models"),
        (
            "gemini_flash",
            "https://generativelanguage.googleapis.com/v1beta/models",
        ),
        ("x_ai", "https://api.x.ai/v1/models"),
        ("deepseek_v3", "https://api.deepseek.com/v1/models"),
        (
            "qwen",
            "https://dashscope.aliyuncs.com/compatible-mode/v1/models",
        ),
        (
            "qwen_us",
            "https://dashscope-us.aliyuncs.com/compatible-mode/v1/models",
        ),
        ("zai", "https://api.z.ai/api/paas/v4/models"),
        ("zai_coding", "https://api.z.ai/api/coding/paas/v4/models"),
        ("zhipu", "https://open.bigmodel.cn/api/paas/v4/models"),
        ("moonshot", "https://api.moonshot.cn/v1/models"),
        ("moonshot_global", "https://api.moonshot.ai/v1/models"),
        ("mistral_ai", "https://api.mistral.ai/v1/models"),
        ("openrouter", "https://openrouter.ai/api/v1/models"),
        ("synthetic", "https://api.synthetic.ai/v1/models"),
        ("together_ai", "https://api.together.xyz/v1/models"),
    ] {
        let endpoint = Url::parse(endpoint).unwrap();
        assert!(
            validate_provider_sync_origin(provider_id, &endpoint).is_ok(),
            "{provider_id} should retain its fixed native sync origin"
        );
    }
    let attacker = Url::parse("https://api.openai.com.attacker.test/v1/models").unwrap();
    assert_eq!(
        validate_provider_sync_origin("openai", &attacker).unwrap_err(),
        "Provider model synchronization origin is not allowlisted."
    );
    let custom = Url::parse("https://custom.example.test/v1/models").unwrap();
    assert!(validate_provider_sync_origin("custom", &custom).is_err());
}

#[test]
fn provider_model_sync_never_forwards_credentials_across_redirects() {
    let redirect_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let credential_sink = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    credential_sink.set_nonblocking(true).unwrap();
    let redirect_address = redirect_listener.local_addr().unwrap();
    let sink_address = credential_sink.local_addr().unwrap();
    let redirect_server = std::thread::spawn(move || {
        let (mut stream, _) = redirect_listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request
            .to_ascii_lowercase()
            .contains("x-api-key: redirect-canary"));
        write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: http://{sink_address}/credential-sink\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
    });

    let client = hardened_provider_sync_client_builder().build().unwrap();
    let response = client
        .get(format!("http://{redirect_address}/models"))
        .header("x-api-key", "redirect-canary")
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    redirect_server.join().unwrap();

    for _ in 0..20 {
        match credential_sink.accept() {
            Ok((mut stream, _)) => {
                let mut request = String::new();
                let _ = stream.read_to_string(&mut request);
                assert!(!request.contains("redirect-canary"));
                panic!("redirect policy connected to a second origin");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("credential sink failed: {error}"),
        }
    }
}

#[test]
fn sprint_59_sse_decoder_handles_split_chunks_and_done_markers() {
    let mut decoder = SseEventDecoder::default();

    assert!(decoder
        .push_chunk("event: content_block_delta\ndata: {\"delta\":{\"text\":\"Hel")
        .unwrap()
        .is_empty());
    assert_eq!(
        decoder.push_chunk("lo\"}}\n\ndata: [DONE]\n\n").unwrap(),
        vec![
            "{\"delta\":{\"text\":\"Hello\"}}".to_string(),
            "[DONE]".to_string()
        ]
    );
    assert!(decoder.finish().unwrap().is_empty());
}

#[test]
fn sprint_59_provider_stream_parsers_extract_text_tokens() {
    let openai = payload_for_provider("openai").unwrap();
    let openai_event = openai.parse_stream_event(&serde_json::json!({
        "id": "chatcmpl-1",
        "choices": [{ "delta": { "content": "Hello" } }]
    }));
    assert_eq!(openai_event.token.as_deref(), Some("Hello"));
    assert_eq!(openai_event.response_id.as_deref(), Some("chatcmpl-1"));

    let anthropic = payload_for_provider("anthropic").unwrap();
    let anthropic_event = anthropic.parse_stream_event(&serde_json::json!({
        "type": "content_block_delta",
        "delta": { "type": "text_delta", "text": "Claude" }
    }));
    assert_eq!(anthropic_event.token.as_deref(), Some("Claude"));

    let gemini = payload_for_provider("gemini").unwrap();
    let gemini_event = gemini.parse_stream_event(&serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [{ "text": "Gemini" }]
            }
        }]
    }));
    assert_eq!(gemini_event.token.as_deref(), Some("Gemini"));
}

#[test]
fn sprint_61_remote_faults_are_classified_and_failover_eligible() {
    let network = InferenceError::network("operation timed out while reading provider stream");
    assert_eq!(network.code, "provider_network_error");
    assert_eq!(network.boundary, "rust_backend");
    assert!(should_attempt_failover(&network, false));

    let rate_limited = InferenceError::provider_rate_limited();
    assert_eq!(rate_limited.code, "provider_rate_limited");
    assert!(should_attempt_failover(&rate_limited, false));

    let credential = InferenceError::credential("invalid API key");
    assert_eq!(credential.code, "credential_unavailable");
    assert_eq!(credential.boundary, "native_keychain");
    assert!(!should_attempt_failover(&credential, false));

    let provider = payload_for_provider("openai").unwrap();
    let empty_payload = provider
        .parse_response(serde_json::json!({
            "choices": [{ "message": { "content": "" } }]
        }))
        .expect_err("empty provider payload should be rejected");
    assert_eq!(empty_payload.code, "provider_response_error");
    assert_eq!(empty_payload.boundary, "provider_api");
    assert!(empty_payload.message.contains("empty response"));
    assert!(!should_attempt_failover(&empty_payload, false));

    let invalid = InferenceError::invalid("Inference prompt cannot be empty.");
    assert!(!should_attempt_failover(&invalid, false));

    let cancelled = InferenceError::local_infer(
        "local_inference_cancelled",
        "Generation was cancelled by the user.",
    );
    assert!(!should_attempt_failover(&cancelled, false));

    let local_loading = InferenceError::local_infer(
        "local_inference_startup_timeout",
        "The local model did not finish loading within 120 seconds.",
    );
    assert!(!should_attempt_failover(&local_loading, true));
}
