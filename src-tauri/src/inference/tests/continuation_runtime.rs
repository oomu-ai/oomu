use super::*;

#[test]
fn dispatch_guard_restores_current_user_turn_when_context_is_empty() {
    let messages = ensure_dispatchable_current_turn(
        vec![InferenceMessage {
            role: "user".to_string(),
            content: String::new(),
            attachments: Vec::new(),
        }],
        "What is going on there?",
        &[],
        "What is going on there?",
    );

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content, "What is going on there?");
}

#[test]
fn dispatch_guard_restores_current_user_turn_when_only_stale_history_exists() {
    let messages = ensure_dispatchable_current_turn(
        vec![InferenceMessage {
            role: "assistant".to_string(),
            content: "Prior assistant answer.".to_string(),
            attachments: Vec::new(),
        }],
        "What is going on there?",
        &[],
        "What is going on there?",
    );

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "assistant");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content, "What is going on there?");
}

#[test]
fn chat_dispatch_audit_segments_include_messages_and_attachment_text_only() {
    let messages = vec![InferenceMessage {
        role: "user".to_string(),
        content: "Review this directory.".to_string(),
        attachments: vec![ChatAttachment {
            name: "Downloads".to_string(),
            mime_type: "text/x-directory-context".to_string(),
            byte_count: 128,
            data_base64: None,
            text: Some("Local Path: /Users/example/Downloads".to_string()),
            approved_file_receipt: None,
        }],
    }];

    let segments = chat_dispatch_audit_segments(&messages);

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].label, "message[0] role=user");
    assert_eq!(segments[0].payload, "Review this directory.");
    assert_eq!(segments[1].label, "message[0] attachment[0] Downloads");
    assert_eq!(segments[1].payload, "Local Path: /Users/example/Downloads");
}

#[test]
fn chat_dispatch_audit_segments_ignore_stale_history_paths() {
    let messages = vec![
        InferenceMessage {
            role: "user".to_string(),
            content: "List /Users/example/Library/Mobile Documents/com~apple~CloudDocs/Eldris"
                .to_string(),
            attachments: Vec::new(),
        },
        InferenceMessage {
            role: "assistant".to_string(),
            content: "notes.md".to_string(),
            attachments: Vec::new(),
        },
        InferenceMessage {
            role: "user".to_string(),
            content: "You do have access to terminal commands.".to_string(),
            attachments: Vec::new(),
        },
    ];

    let segments = chat_dispatch_audit_segments(&messages);

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].label, "message[2] role=user");
    assert_eq!(
        segments[0].payload,
        "You do have access to terminal commands."
    );
    audit_workspace_execution_payload_segments(&segments)
        .expect("stale history path should not block this turn");
}

#[test]
fn approved_oomu_plan_can_analyze_eldris_workspace_language() {
    let messages = vec![InferenceMessage {
        role: "user".to_string(),
        content: "Review the attached OOMU remediation plan and summarize it.".to_string(),
        attachments: vec![ChatAttachment {
            name: "oomu_reliability_remediation_plan.md".to_string(),
            mime_type: "text/markdown".to_string(),
            byte_count: 96,
            data_base64: None,
            text: Some(
                "This OOMU plan compares the Eldris Workspace boundary without requesting access."
                    .to_string(),
            ),
            approved_file_receipt: None,
        }],
    }];

    let segments = chat_dispatch_audit_segments(&messages);
    audit_workspace_execution_payload_segments(&segments)
        .expect("approved analytical attachment content must remain dispatchable");
}

#[test]
fn local_chat_prompt_segments_search_grounding_attachments() {
    let search_context = concat!(
            "Local Web Search Context\n",
            "Query: latest OOMU release\n",
            "Engine: duckduckgo_lite_static\n\n",
            "[{\"title\":\"Release notes\",\"url\":\"https://example.com\",\"snippet\":\"Fresh facts.\"}]"
        );
    let messages = vec![InferenceMessage {
        role: "user".to_string(),
        content: message_with_attachment_receipt(
            "What changed?",
            &[ChatAttachment {
                name: "local_web_search.md".to_string(),
                mime_type: "text/markdown".to_string(),
                byte_count: search_context.len(),
                data_base64: None,
                text: Some(search_context.to_string()),
                approved_file_receipt: None,
            }],
        ),
        attachments: vec![ChatAttachment {
            name: "local_web_search.md".to_string(),
            mime_type: "text/markdown".to_string(),
            byte_count: search_context.len(),
            data_base64: None,
            text: Some(search_context.to_string()),
            approved_file_receipt: None,
        }],
    }];

    let settings = runtime_settings_for_reasoning(Some("medium"));
    let prompt = format_local_chat_prompt("session-test", "", &messages, None, &settings);
    let parsed: serde_json::Value = serde_json::from_str(&prompt).unwrap();
    let content = parsed["messages"][0]["content"].as_str().unwrap();

    assert!(content.contains("Read-only factual search grounding attached for this turn."));
    assert!(content.contains(grounding_contract::HEADER));
    assert!(content.contains(grounding_contract::DIRECTIVE));
    assert!(content.contains("Verified public-source evidence"));
    assert!(!content.contains("Local Web Search Context"));
    assert!(content.contains("Query: latest OOMU release"));
    assert!(content.contains("Engine: duckduckgo_lite_static"));
    assert!(content.contains("Fresh facts."));
    assert!(content.contains("every requested subject, field, distinction, comparison"));
    assert!(!content.contains("local_web_search.md"));
    assert!(!content.contains("Text excerpt: Local Web Search Context"));
    assert!(!content.contains("text content:\nLocal Web Search Context"));
}

#[test]
fn public_search_grounding_round_trips_through_hidden_turn_metadata() {
    let search_context = concat!(
        "Local Web Search Context\n",
        "Query: ROC to SIN\n",
        "Engine: mod_declared_public_context\n\n",
        "Verified itinerary facts from the approved public source."
    );
    let public_grounding = ChatAttachment {
        name: "local_web_search.md".to_string(),
        mime_type: "text/markdown".to_string(),
        byte_count: search_context.len(),
        data_base64: None,
        text: Some(search_context.to_string()),
        approved_file_receipt: None,
    };
    let private_attachment = ChatAttachment {
        name: "private_notes.txt".to_string(),
        mime_type: "text/plain".to_string(),
        byte_count: 19,
        data_base64: None,
        text: Some("private trip notes".to_string()),
        approved_file_receipt: None,
    };

    let persisted =
        persisted_public_grounding_attachments(&[public_grounding.clone(), private_attachment]);
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].name, public_grounding.name);
    assert_eq!(persisted[0].text, public_grounding.text);

    let metadata = serde_json::json!({
        (PUBLIC_GROUNDING_METADATA_KEY): persisted,
    })
    .to_string();
    let restored = public_grounding_attachments_from_metadata(Some(&metadata));
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].name, public_grounding.name);
    assert_eq!(restored[0].text, public_grounding.text);
}

#[test]
fn hidden_turn_metadata_rejects_untrusted_or_binary_attachment_payloads() {
    let forged = serde_json::json!({
        (PUBLIC_GROUNDING_METADATA_KEY): [{
            "name": "local_web_search.md",
            "mime_type": "text/markdown",
            "byte_count": 16,
            "data_base64": "cHJpdmF0ZQ==",
            "text": "Local Web Search Context\nprivate",
            "approved_file_receipt": null
        }]
    })
    .to_string();

    assert!(public_grounding_attachments_from_metadata(Some(&forged)).is_empty());
    assert!(public_grounding_attachments_from_metadata(Some("not-json")).is_empty());
}

#[test]
fn local_chat_prompt_omits_empty_leaked_channel_history() {
    let messages = vec![InferenceMessage {
        role: "assistant".to_string(),
        content: "<|channel>thought\n<channel|><|channel>thought\n<channel|>".to_string(),
        attachments: Vec::new(),
    }];

    let settings = runtime_settings_for_reasoning(Some("medium"));
    let prompt = format_local_chat_prompt("session-test", "", &messages, None, &settings);
    let parsed: serde_json::Value = serde_json::from_str(&prompt).unwrap();

    assert_eq!(parsed["messages"][0]["role"], "assistant");
    assert_eq!(
        parsed["messages"][0]["content"],
        "<|channel>thought\n<channel|><|channel>thought\n<channel|>"
    );
}

#[test]
fn stream_text_sanitizer_extracts_only_text_payloads() {
    let streamed = concat!(
        "data: 0:\"Hello\\n\"\n",
        "0:\"world\"\n",
        "e:{\"finishReason\":\"stop\"}\n",
        "d:{\"finishReason\":\"stop\"}\n",
    );

    assert_eq!(sanitize_stream_text(streamed), "Hello\nworld");
}

#[test]
fn stream_text_sanitizer_preserves_word_boundaries_across_chunks() {
    let streamed = concat!(
        "data: 0:\"Give\"\n",
        "data: 0:\"me\"\n",
        "data: 0:\"space\"\n",
        "d:{\"finishReason\":\"stop\"}\n",
    );

    assert_eq!(sanitize_stream_text(streamed), "Give me space");
}

#[test]
fn stream_text_merger_repairs_oomu_word_boundary_regression() {
    let chunks = [
        "I am writing this response to see if the",
        "spacing between words remains intact.",
        " That merging suggests",
        "there is still an edge case. ",
        "In",
        "tokenization, spaces can be attached to tokens. It might",
        "still get stripped and strips",
        "leading whitespace. We are almost",
        "there.",
    ];

    let text =
        stream_text::merge_stream_text_chunks(chunks.into_iter().map(|chunk| chunk.to_string()));

    assert!(text.contains("the spacing"));
    assert!(text.contains("suggests there"));
    assert!(text.contains("In tokenization"));
    assert!(text.contains("might still"));
    assert!(text.contains("strips leading"));
    assert!(text.contains("almost there"));
}

#[test]
fn stream_text_merger_keeps_common_subword_fragments_joined() {
    let chunks = [
        "Hel", "lo", " ", "com", "pletely", " ", "render", "ing", " ", "config", "uration",
    ];

    assert_eq!(
        stream_text::merge_stream_text_chunks(chunks.into_iter().map(|chunk| chunk.to_string()),),
        "Hello completely rendering configuration"
    );
}

#[test]
fn stream_text_sanitizer_preserves_punctuation_boundary_chunks() {
    let streamed = concat!(
        "data: 0:\"Hello.\"\n",
        "data: 0:\" \"\n",
        "data: 0:\"World.\"\n",
        "d:{\"finishReason\":\"stop\"}\n",
    );

    assert_eq!(sanitize_stream_text(streamed), "Hello. World.");
}

#[test]
fn stream_text_sanitizer_preserves_multiline_paragraph_chunks() {
    let streamed = concat!(
        "data: 0:\"Line 1.\\n\"\n",
        "data: 0:\"\\nLine 2.\"\n",
        "d:{\"finishReason\":\"stop\"}\n",
    );

    assert_eq!(sanitize_stream_text(streamed), "Line 1.\n\nLine 2.");
}

#[test]
fn stream_text_sanitizer_preserves_word_boundaries_around_channel_tags() {
    let streamed = concat!(
        "data: 0:\"Apply this to\"\n",
        "data: 0:\"<|channel>text\\n<channel|>this file, then tell\"\n",
        "data: 0:\"<|channel>final<channel|>me what changed.\"\n",
        "d:{\"finishReason\":\"stop\"}\n",
    );

    assert_eq!(
        sanitize_stream_text(streamed),
        "Apply this to this file, then tell me what changed."
    );
}

#[test]
fn stream_text_sanitizer_unwraps_serialized_protocol_arrays() {
    let streamed = r#"["0:\"Clean \"","0:\"history\"","d:{\"finishReason\":\"stop\"}"]"#;

    assert_eq!(sanitize_stream_text(streamed), "Clean history");
}

#[test]
fn stream_text_sanitizer_filters_control_tokens_split_across_chunks() {
    let streamed = concat!(
        "0:\"<|chan\"\n",
        "0:\"nel>thought\\n<channel|>private reasoning\\n\"\n",
        "0:\"<|channel>text\\n<channel|>Visible **answer**.<turn|>\"\n",
        "d:{\"finishReason\":\"stop\"}\n",
    );

    assert_eq!(sanitize_stream_text(streamed), "Visible **answer**.");
}

#[test]
fn stream_text_sanitizer_filters_ministral_markup_variants() {
    let streamed = concat!(
        "0:\"The NASDAQ is down\"\n",
        "0:\"</chan\"\n",
        "0:\"nel> <|mod\"\n",
        "0:\"el>slightly.\"\n",
        "d:{\"finishReason\":\"stop\"}\n",
    );

    assert_eq!(
        sanitize_stream_text(streamed),
        "The NASDAQ is down slightly."
    );
}

#[test]
fn stream_text_sanitizer_preserves_plain_model_output() {
    let plain = "0 reasons to keep protocol markers in this answer.";

    assert_eq!(sanitize_stream_text(plain), plain);
}

#[test]
fn local_infer_stderr_parser_distinguishes_tokens_and_exact_errors() {
    assert!(matches!(
        parse_local_infer_stderr_record(r#"{"event":"ready","sequence":0,"elapsed_ms":0}"#),
        LocalInferStderrRecord::Ready
    ));
    assert!(matches!(
        parse_local_infer_stderr_record(r#"{"event":"progress","sequence":7,"elapsed_ms":2048}"#),
        LocalInferStderrRecord::Progress
    ));
    assert!(matches!(
        parse_local_infer_stderr_record(
            r#"{"event":"token","sequence":1,"elapsed_ms":412,"token":"Hello"}"#
        ),
        LocalInferStderrRecord::Token(LocalInferToken {
            sequence: 1,
            token,
        }) if token == "Hello"
    ));

    let error = local_infer_error_payload(
        "LOCAL_INFER_STATEFUL_GGUF_FALLBACK requested=x resolved=y\n\
             {\"code\":\"gemma_asset_missing\",\"message\":\"Tokenizer not found.\"}",
    )
    .expect("structured helper error");
    assert_eq!(error.code, "gemma_asset_missing");
    assert_eq!(error.message, "Tokenizer not found.");
}

#[test]
fn expanded_terminal_contract_rejects_the_previous_helper_protocol() {
    assert_eq!(LOCAL_INFER_PROTOCOL_VERSION, 8);
    validate_local_infer_protocol_version("8").expect("current helper is accepted");
    let error = validate_local_infer_protocol_version("7")
        .expect_err("old helper cannot satisfy the expanded terminal contract");
    assert!(error.message.contains("requires 8"));
    assert!(error.message.contains("reports 7"));
}

#[test]
fn local_helper_startup_has_one_bounded_two_minute_attempt() {
    assert_eq!(LOCAL_INFER_STARTUP_TIMEOUT, Duration::from_secs(120));
    assert_eq!(
        classify_inference_error(&InferenceError::local_infer(
            "local_inference_startup_timeout",
            "startup expired"
        )),
        InferenceFailureClass::Fatal
    );
}

#[test]
fn only_a_cloud_semantic_route_schedules_the_exact_local_baseline() {
    assert!(local_prewarm::should_schedule(
        "cloud_tier_2",
        "gemma-4-12B-it-qat-q4_0-gguf"
    ));
    assert!(!local_prewarm::should_schedule(
        "local_tier_1",
        "gemma-4-12B-it-qat-q4_0-gguf"
    ));
    assert!(!local_prewarm::should_schedule("cloud_tier_2", "  "));
}

#[test]
fn local_infer_error_keeps_helper_code_and_boundary() {
    let error = local_infer_error(
        r#"{"code":"local_model_repetition_collapse","message":"Retry with another model."}"#,
        None,
    );

    assert_eq!(error.code, "local_model_repetition_collapse");
    assert_eq!(error.boundary, "local_infer");
    assert_eq!(error.message, "Retry with another model.");
}

#[test]
fn local_model_idle_timeout_is_bounded_and_defaults_to_five_minutes() {
    assert_eq!(
        parse_local_model_idle_timeout(None),
        Duration::from_secs(300)
    );
    assert_eq!(
        parse_local_model_idle_timeout(Some("1")),
        Duration::from_secs(5)
    );
    assert_eq!(
        parse_local_model_idle_timeout(Some("999999")),
        Duration::from_secs(24 * 60 * 60)
    );
    assert_eq!(
        parse_local_model_idle_timeout(Some("invalid")),
        Duration::from_secs(300)
    );
}

#[test]
fn local_stream_cancellation_is_scoped_and_clearable() {
    let stream_id = format!("cancel-test-{}", std::process::id());
    clear_local_stream_cancellation(&stream_id);
    assert!(!is_local_stream_cancelled(Some(&stream_id)));
    assert!(cancel_chat_stream(stream_id.clone()));
    assert!(is_local_stream_cancelled(Some(&stream_id)));
    assert!(!is_local_stream_cancelled(Some("another-stream")));
    clear_local_stream_cancellation(&stream_id);
    assert!(!is_local_stream_cancelled(Some(&stream_id)));
}

#[cfg(unix)]
#[test]
fn local_infer_reaper_force_stops_and_waits_for_a_lingering_process() {
    let child = Command::new("sh")
        .args(["-c", "while :; do sleep 1; done"])
        .spawn()
        .expect("lingering test process should start");
    let started = Instant::now();

    reap_local_infer_child(child, None, None);

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "forced local inference cleanup exceeded its bounded grace period"
    );
}

#[tokio::test]
async fn test_preflight_route_classification_detects_local_action_intent() {
    let standard_req = crate::agentic_loop::ChatIntentRouteRequest {
        prompt: "Hello, OOMU, how are you today?".to_string(),
        automated_web_grounding_enabled: None,
        attachments: Vec::new(),
    };
    let standard_decision = crate::agentic_loop::classify_chat_intent_route_inner(standard_req)
        .await
        .unwrap();
    assert!(matches!(
        standard_decision.route,
        crate::agentic_loop::ChatIntentRoute::ConversationalStream
    ));

    let action_req = crate::agentic_loop::ChatIntentRouteRequest {
        prompt:
            "Please look at Project OOMU files at /Users/example/OOMU and edit the gemma.rs file"
                .to_string(),
        automated_web_grounding_enabled: None,
        attachments: Vec::new(),
    };
    let action_decision = crate::agentic_loop::classify_chat_intent_route_inner(action_req)
        .await
        .unwrap();
    assert!(matches!(
        action_decision.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ));
    assert!(action_decision.requires_local_access);
}
