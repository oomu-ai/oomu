use super::*;
use crate::agent_manager::ConfiguredProvider;

mod approved_file_receipts;
mod provider_contracts;

fn test_message(role: &str, content: &str) -> InferenceMessage {
    InferenceMessage {
        role: role.to_string(),
        content: content.to_string(),
        attachments: Vec::new(),
    }
}

fn test_attachment(name: &str, byte_count: usize) -> ChatAttachment {
    ChatAttachment {
        name: name.to_string(),
        mime_type: "text/plain".to_string(),
        byte_count,
        data_base64: None,
        text: Some("test attachment payload".to_string()),
        approved_file_receipt: None,
    }
}

fn identity_context_with_memories(
    memories: Vec<crate::memory_ledger::AgentMemoryEntry>,
) -> AgentIdentityContext {
    AgentIdentityContext {
        agent_id: "agent-memory-test".to_string(),
        soul: crate::memory_ledger::AgentSoulManifest {
            agent_id: "agent-memory-test".to_string(),
            display_name: "OOMU".to_string(),
            origin_story: String::new(),
            role: "Assistant".to_string(),
            values: Vec::new(),
            hard_boundaries: Vec::new(),
            communication_style: "Clear".to_string(),
            self_description: String::new(),
            immutable_truths: Vec::new(),
            version: 1,
            signature: crate::sovereign_identity::SignatureBlock::default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        },
        memories,
        user_profile: None,
        path_context: None,
        prompt_context: String::new(),
        secure_memory_available: true,
    }
}

#[test]
fn project_cloud_consent_uses_a_stable_resumable_error_contract() {
    let error = InferenceError::project_provider_consent_required();
    assert_eq!(error.code, "project_provider_consent_required");
    assert_eq!(error.boundary, "project_policy_preflight");
}

#[test]
fn project_local_only_block_has_a_distinct_actionable_error_contract() {
    let error = InferenceError::project_provider_blocked();
    assert_eq!(error.code, "project_provider_blocked");
    assert_eq!(error.boundary, "project_policy");
    assert!(error.message.contains("Choose a local model"));
}

#[test]
fn project_cloud_confirmation_is_bound_to_one_exact_turn_and_route() {
    let suffix = native_chat_turn_identity("project-confirmation-test");
    let session_id = format!("session-{suffix}");
    let turn_id = format!("turn-{suffix}");
    let generation_token = format!("generation-{suffix}");
    let project_id = format!("project-{suffix}");
    register_project_provider_confirmation_challenge(
        &session_id,
        &turn_id,
        &generation_token,
        &project_id,
        "prov-reviewed-cloud",
        "google_gemini",
    );

    assert!(!consume_project_provider_confirmation_challenge(
        &session_id,
        &turn_id,
        &generation_token,
        &project_id,
        "prov-different-cloud",
        "google_gemini",
    ));
    assert!(consume_project_provider_confirmation_challenge(
        &session_id,
        &turn_id,
        &generation_token,
        &project_id,
        "prov-reviewed-cloud",
        "google_gemini",
    ));
    assert!(!consume_project_provider_confirmation_challenge(
        &session_id,
        &turn_id,
        &generation_token,
        &project_id,
        "prov-reviewed-cloud",
        "google_gemini",
    ));
}

#[test]
fn project_cloud_confirmation_request_field_is_optional_and_camel_case() {
    let absent = serde_json::from_value::<ChatTurnRequest>(serde_json::json!({
        "agent_id": "agent-test",
        "message": "hello"
    }))
    .unwrap();
    assert_eq!(absent.project_cloud_confirmed, None);

    let confirmed = serde_json::from_value::<ChatTurnRequest>(serde_json::json!({
        "agent_id": "agent-test",
        "message": "hello",
        "projectCloudConfirmed": true
    }))
    .unwrap();
    assert_eq!(confirmed.project_cloud_confirmed, Some(true));
}

#[test]
fn native_execution_receipt_id_is_optional_and_accepts_camel_case() {
    let absent = serde_json::from_value::<ChatTurnRequest>(serde_json::json!({
        "agent_id": "agent-test",
        "message": "hello"
    }))
    .unwrap();
    assert_eq!(absent.native_execution_receipt_id, None);

    let bound = serde_json::from_value::<ChatTurnRequest>(serde_json::json!({
        "agent_id": "agent-test",
        "message": "continue",
        "nativeExecutionReceiptId": "apple-operation-1234-abcd-1"
    }))
    .unwrap();
    assert_eq!(
        bound.native_execution_receipt_id.as_deref(),
        Some("apple-operation-1234-abcd-1")
    );
}

#[test]
fn private_egress_provider_boundary_blocks_before_cloud_client_construction() {
    let (agent_manager, manager_db_path) = temporary_agent_manager("private-egress-boundary");
    let persistence_path = std::env::temp_dir().join(format!(
        "oomu-private-egress-boundary-{}-{}.sqlite",
        std::process::id(),
        unix_time_ms()
    ));
    let persistence = PersistenceEngine::initialize_at(persistence_path.clone()).unwrap();
    let route = ResolvedProviderRoute {
        route_provider_id: "openai".to_string(),
        catalog_provider_id: "openai".to_string(),
        overrides: ProviderRouteOverrides {
            base_url: Some("http://127.0.0.1:1/v1".to_string()),
            api_key_label: None,
            api_key: None,
        },
    };
    let messages = vec![InferenceMessage {
        role: "user".to_string(),
        content: "Summarize this privately.".to_string(),
        attachments: vec![test_attachment("local_contacts.json", 23)],
    }];

    let error = execute_chat_inference_with_failover(
        &route,
        "test-model",
        "session-1",
        "turn-1",
        "system",
        &messages,
        &PathBuf::new(),
        None,
        "standard",
        None,
        &agent_manager,
        &persistence,
        false,
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "private_egress_receipt_required");

    let _ = std::fs::remove_file(manager_db_path);
    let _ = std::fs::remove_file(persistence_path);
}

#[test]
fn native_attachment_validator_rejects_missing_payloads() {
    let missing_payload = vec![ChatAttachment {
        text: Some("   ".to_string()),
        data_base64: Some("\n\t".to_string()),
        ..test_attachment("empty.txt", 0)
    }];

    assert_eq!(
        validate_chat_attachments(&missing_payload),
        Err("attachment_payload_missing")
    );
}

#[test]
fn native_attachment_validator_enforces_request_wide_provider_limits() {
    let request = InferenceRequest {
        provider_id: "openai".to_string(),
        model_id: "test-model".to_string(),
        system_prompt: None,
        messages: vec![
            InferenceMessage {
                role: "user".to_string(),
                content: "first".to_string(),
                attachments: (0..3)
                    .map(|index| test_attachment(&format!("first-{index}.txt"), 1))
                    .collect(),
            },
            InferenceMessage {
                role: "user".to_string(),
                content: "second".to_string(),
                attachments: (0..3)
                    .map(|index| test_attachment(&format!("second-{index}.txt"), 1))
                    .collect(),
            },
        ],
        prompt: None,
        temperature: None,
        max_tokens: None,
        reasoning: None,
        reasoning_budget_tokens: None,
        base_url: None,
        api_key_label: None,
        api_key: None,
    };

    let error = validate_inference_request_attachments(&request).unwrap_err();
    assert_eq!(error.code, "invalid_request");
    assert_eq!(error.message, "attachment_count_limit_exceeded");
}

#[test]
fn native_attachment_validator_enforces_count_and_decoded_aggregate_limits() {
    let too_many = (0..=MAX_CHAT_ATTACHMENTS)
        .map(|index| test_attachment(&format!("file-{index}.txt"), 0))
        .collect::<Vec<_>>();
    assert_eq!(
        validate_chat_attachments(&too_many),
        Err("attachment_count_limit_exceeded")
    );

    let oversized_file = vec![test_attachment(
        "oversized.txt",
        MAX_CHAT_ATTACHMENT_FILE_BYTES + 1,
    )];
    assert_eq!(
        validate_chat_attachments(&oversized_file),
        Err("attachment_file_byte_limit_exceeded")
    );

    let decoded_overflow = (0..3)
        .map(|index| test_attachment(&format!("large-{index}.txt"), 7 * 1024 * 1024))
        .collect::<Vec<_>>();
    assert_eq!(
        validate_chat_attachments(&decoded_overflow),
        Err("attachment_aggregate_byte_limit_exceeded")
    );
}

#[test]
fn native_attachment_validator_enforces_encoded_and_text_limits() {
    let encoded_chunk = "A".repeat(MAX_CHAT_ATTACHMENT_ENCODED_BYTES / 5 + 1);
    let encoded_overflow = (0..5)
        .map(|index| ChatAttachment {
            data_base64: Some(encoded_chunk.clone()),
            ..test_attachment(&format!("encoded-{index}.txt"), 0)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        validate_chat_attachments(&encoded_overflow),
        Err("attachment_encoded_byte_limit_exceeded")
    );

    let text_overflow = vec![ChatAttachment {
        text: Some("x".repeat(MAX_CHAT_ATTACHMENT_TEXT_BYTES + 1)),
        ..test_attachment("oversized-text.txt", 1)
    }];
    assert_eq!(
        validate_chat_attachments(&text_overflow),
        Err("attachment_text_byte_limit_exceeded")
    );

    let aggregate_text_overflow = (0..5)
        .map(|index| ChatAttachment {
            text: Some("x".repeat(220 * 1024)),
            ..test_attachment(&format!("text-{index}.txt"), 1)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        validate_chat_attachments(&aggregate_text_overflow),
        Err("attachment_text_aggregate_byte_limit_exceeded")
    );
}

#[test]
fn native_attachment_validator_checks_base64_length_and_declared_bytes() {
    let mismatched = vec![ChatAttachment {
        byte_count: 4,
        data_base64: Some(general_purpose::STANDARD.encode(b"hello")),
        ..test_attachment("payload.txt", 0)
    }];
    assert_eq!(
        validate_chat_attachments(&mismatched),
        Err("attachment_byte_count_mismatch")
    );

    let malformed = vec![ChatAttachment {
        data_base64: Some("not-base64".to_string()),
        ..test_attachment("malformed.txt", 0)
    }];
    assert_eq!(
        validate_chat_attachments(&malformed),
        Err("attachment_base64_invalid")
    );
}

#[test]
fn native_attachment_validator_rejects_pixel_bombs() {
    let mut png_header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png_header.extend_from_slice(&7_000_u32.to_be_bytes());
    png_header.extend_from_slice(&7_000_u32.to_be_bytes());
    let pixel_bomb = vec![ChatAttachment {
        name: "pixel-bomb.png".to_string(),
        mime_type: "image/png".to_string(),
        byte_count: png_header.len(),
        data_base64: Some(general_purpose::STANDARD.encode(&png_header)),
        text: None,
        approved_file_receipt: None,
    }];

    assert_eq!(
        validate_chat_attachments(&pixel_bomb),
        Err("attachment_image_dimension_limit_exceeded")
    );
}

fn temporary_agent_manager(test_name: &str) -> (AgentManager, PathBuf) {
    let now = unix_time_ms();
    let db_path = std::env::temp_dir().join(format!(
        "oomu-inference-{test_name}-{}-{now}.db",
        std::process::id()
    ));
    let manager =
        AgentManager::initialize_at(db_path.clone()).expect("temporary agent manager initializes");
    (manager, db_path)
}

fn configured_provider(id: &str, provider_id: &str, model_ids: &str) -> ConfiguredProvider {
    let base_url = match provider_id {
        "openrouter" => "https://openrouter.ai/api/v1",
        "openai" => "https://api.openai.com/v1",
        "custom" => "https://custom.example.test/v1",
        _ => "https://api.example.test/v1",
    };
    ConfiguredProvider {
        id: id.to_string(),
        provider_id: provider_id.to_string(),
        provider_name: provider_id.to_string(),
        auth_method: "api_key".to_string(),
        base_url: base_url.to_string(),
        api_key_label: "TEST_API_KEY".to_string(),
        api_key: None,
        credential_configured: false,
        custom_model_ids: model_ids.to_string(),
        auto_route_target: false,
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}

fn conversational_capability(
    server_name: &str,
    tool_name: &str,
) -> ConversationalMcpToolCapability {
    ConversationalMcpToolCapability {
        server_name: server_name.to_string(),
        tool_name: tool_name.to_string(),
        description: format!("{server_name}/{tool_name}"),
        input_schema: serde_json::json!({"type":"object"}),
    }
}

fn conversational_capability_keys(capabilities: &[ConversationalMcpToolCapability]) -> Vec<String> {
    capabilities
        .iter()
        .map(|capability| format!("{}/{}", capability.server_name, capability.tool_name))
        .collect()
}

#[test]
fn conversational_mcp_contract_includes_local_utility_read_tools() {
    let capabilities = vec![
        conversational_capability("macos_applescript", "add_system_reminder"),
        conversational_capability("macos_applescript", "create_system_note"),
        conversational_capability("macos_applescript", "draft_system_email"),
        conversational_capability("macos_applescript", "read_apple_app_ui"),
        conversational_capability("macos_applescript", "read_system_calendar"),
        conversational_capability("macos_applescript", "read_system_contacts"),
        conversational_capability("macos_applescript", "read_system_emails"),
        conversational_capability("macos_applescript", "read_system_music"),
        conversational_capability("macos_applescript", "read_system_notes"),
    ];

    let contract = conversational_mcp_tool_contract(&capabilities)
        .expect("local utility read tools should be advertised");

    assert!(contract.contains("macos_applescript/read_system_calendar"));
    assert!(contract.contains("bounded start_date and end_date"));
    assert!(contract.contains("macos_applescript/read_system_notes"));
    assert!(contract.contains("macos_applescript/read_system_contacts"));
    assert!(contract.contains("macos_applescript/read_apple_app_ui"));
    assert!(contract.contains("macos_applescript/add_system_reminder"));
    assert!(contract.contains("macos_applescript/create_system_note"));
    assert!(contract.contains("explicit user approval"));
    assert!(contract.contains("macos_applescript/read_system_emails"));
    assert!(contract.contains("macos_applescript/read_system_music"));
    assert!(contract.contains("never starts playback"));
    assert!(contract.contains("unread_only true"));
    assert!(has_connected_conversational_mcp_tools(&capabilities));
}

#[test]
fn conversational_mcp_contract_exposes_connected_catalog_without_a_static_allowlist() {
    let capabilities = vec![conversational_capability(
        "connected_customer_service",
        "lookup_customer",
    )];

    let contract = conversational_mcp_tool_contract(&capabilities)
        .expect("a connected catalog entry should be available for model selection");
    assert!(contract.contains("connected_customer_service/lookup_customer"));
    assert!(has_connected_conversational_mcp_tools(&capabilities));
}

#[test]
fn conversational_mcp_contract_truthfully_routes_current_public_facts_to_search() {
    let capabilities = vec![conversational_capability("local_search", "search_web")];

    let contract = conversational_mcp_tool_contract(&capabilities)
        .expect("the connected public-search tool should be advertised");

    assert!(contract.contains("local_search/search_web"));
    assert!(contract.contains("current or changing public facts"));
    assert!(contract.contains("Do not answer from model memory, training data"));
    assert!(contract.contains("native broker will decide"));
    assert!(!contract.contains("do not enable web search"));
}

#[test]
fn public_freshness_boundary_accepts_only_an_exact_search_tool_request() {
    let valid = r#"```oomu_mcp_tool_call
{"serverName":"local_search","toolName":"search_web","arguments":{"query":"latest edition Writing AI Prompts for Dummies","max_results":5}}
```"#;
    assert!(is_exact_public_web_search_tool_request(valid));
    assert!(is_exact_public_web_search_tool_request(
        &valid.replace("```oomu_mcp_tool_call", "```json oomu_mcp_tool_call")
    ));

    for rejected in [
        "The latest edition is the 3rd Edition.",
        r#"I think it is the 3rd Edition.
```oomu_mcp_tool_call
{"serverName":"local_search","toolName":"search_web","arguments":{"query":"latest edition Writing AI Prompts for Dummies"}}
```"#,
        r#"```oomu_mcp_tool_call
{"serverName":"local_filesystem","toolName":"read_file","arguments":{"path":"answer.txt"}}
```"#,
        r#"```oomu_mcp_tool_call
{"serverName":"local_search","toolName":"search_web","arguments":{"query":""}}
```"#,
        r#"```oomu_mcp_tool_call
{"serverName":"local_search","toolName":"search_web","arguments":{"query":"book edition","privateContext":"calendar"}}
```"#,
        r#"```oomu_mcp_tool_call
{"serverName":"local_search","toolName":"search_web","arguments":{"query":"book\nedition"}}
```"#,
        r#"```oomu_mcp_tool_call
{"serverName":"local_search","toolName":"search_web","arguments":{"query":"book edition","max_results":6}}
```"#,
    ] {
        assert!(
            !is_exact_public_web_search_tool_request(rejected),
            "unexpectedly accepted: {rejected}"
        );
    }
}

#[test]
fn public_freshness_boundary_builds_a_canonical_escaped_search_fallback() {
    let capabilities = vec![conversational_capability("local_search", "search_web")];
    let user_message = format!(
        "What’s the latest \"Writing AI Prompts\" edition?\nPublisher: Wiley {}",
        "界".repeat(520)
    );

    let (replacement, finish_reason) = public_web_search_boundary_replacement(
        &user_message,
        "The latest edition is the 3rd Edition.",
        &capabilities,
    )
    .expect("ungrounded prose should become a canonical search request");

    assert_eq!(finish_reason, "canonical_web_search_request");
    assert!(is_exact_public_web_search_tool_request(&replacement));
    assert!(replacement.contains("\\\"Writing AI Prompts\\\""));
    assert!(!replacement.contains("\\n"));
    let payload = replacement
        .strip_prefix("```oomu_mcp_tool_call\n")
        .and_then(|value| value.strip_suffix("\n```"))
        .expect("canonical fence");
    let value: serde_json::Value = serde_json::from_str(payload).expect("canonical JSON");
    let query = value["arguments"]["query"].as_str().expect("bounded query");
    assert!(query.chars().count() <= 500);
    assert!(query.contains("edition? Publisher: Wiley"));
    assert!(!query.ends_with('界'));
    assert_eq!(value["arguments"]["max_results"], 5);

    assert!(
        public_web_search_boundary_replacement(&user_message, &replacement, &capabilities,)
            .is_none()
    );
}

#[test]
fn public_freshness_boundary_preserves_a_bounded_model_query_for_native_approval() {
    let capabilities = vec![conversational_capability("local_search", "search_web")];
    let user_message = "What’s the latest edition of the book “Writing AI Prompts for Dummies”?";
    let model_output = r#"```oomu_mcp_tool_call
{"serverName":"local_search","toolName":"search_web","arguments":{"query":"Writing AI Prompts for Dummies latest edition reviews 2026","max_results":5}}
```"#;

    assert!(
        public_web_search_boundary_replacement(user_message, model_output, &capabilities,)
            .is_none()
    );
}

#[test]
fn public_freshness_boundary_uses_an_honest_deficit_when_search_is_unavailable() {
    let (replacement, finish_reason) = public_web_search_boundary_replacement(
        "What’s the latest edition?",
        "The latest edition is the 3rd Edition.",
        &[],
    )
    .expect("missing native search capability must fail closed");

    assert_eq!(replacement, PUBLIC_WEB_VERIFICATION_REQUIRED);
    assert_eq!(finish_reason, "web_verification_required");
    assert!(!replacement.contains("3rd Edition"));
    assert!(!replacement.contains("found public evidence"));
    assert!(replacement.contains("Public search isn’t available right now"));
    assert!(!replacement.contains("request your approval"));
}

#[test]
fn connected_mcp_tools_remain_visible_for_informational_routes() {
    let capabilities = vec![conversational_capability(
        "macos_applescript",
        "read_system_emails",
    )];
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
        requires_local_access: false,
        decision_source: "contextual_informational_topic_filter".to_string(),
        reason: "informational".to_string(),
        matched_signals: Vec::new(),
        status_label: "OOMU is typing...".to_string(),
    };

    let filtered = filter_conversational_mcp_tool_capabilities_for_turn(
        &capabilities,
        &[],
        &decision,
        "How does email work?",
    );

    assert_eq!(
        conversational_capability_keys(&filtered),
        conversational_capability_keys(&capabilities)
    );
}

#[tokio::test]
async fn access_my_calendar_keeps_the_connected_calendar_schema_available() {
    let capabilities = vec![conversational_capability(
        "macos_applescript",
        "read_system_calendar",
    )];
    let decision = crate::agentic_loop::classify_chat_intent_route_inner(
        crate::agentic_loop::ChatIntentRouteRequest {
            prompt: "Access my calendar.".to_string(),
            automated_web_grounding_enabled: Some(false),
            attachments: Vec::new(),
        },
    )
    .await
    .expect("the native route classifier should accept the request");
    let decision = enforce_backend_executable_intent_gate(decision, "Access my calendar.", &[]);
    assert!(!executable_intent_gate::requires_agentic_escalation(
        &decision,
        "Access my calendar.",
        &capabilities,
    ));

    let filtered = filter_conversational_mcp_tool_capabilities_for_turn(
        &capabilities,
        &[],
        &decision,
        "Access my calendar.",
    );

    assert_eq!(
        conversational_capability_keys(&filtered),
        vec!["macos_applescript/read_system_calendar"]
    );
}

#[test]
fn direct_mail_read_keeps_the_connected_catalog_for_model_selection() {
    let capabilities = vec![
        conversational_capability("macos_applescript", "read_system_emails"),
        conversational_capability("macos_applescript", "draft_system_email"),
        conversational_capability("macos_applescript", "read_system_calendar"),
        conversational_capability("local_filesystem", "read_file"),
    ];
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "private_app_data_filter".to_string(),
        reason: "private mail read".to_string(),
        matched_signals: Vec::new(),
        status_label: "Checking Mail".to_string(),
    };

    let filtered = filter_conversational_mcp_tool_capabilities_for_turn(
        &capabilities,
        &[],
        &decision,
        "Do I have any unread emails?",
    );

    assert_eq!(
        conversational_capability_keys(&filtered),
        conversational_capability_keys(&capabilities)
    );
}

#[test]
fn workspace_mail_attachment_blocks_equivalent_mail_reader() {
    let capabilities = vec![
        conversational_capability("macos_applescript", "read_system_emails"),
        conversational_capability("macos_applescript", "read_system_calendar"),
        conversational_capability("local_filesystem", "read_file"),
    ];
    let attachments = vec![ChatAttachment {
        name: "local_mail.json".to_string(),
        mime_type: "application/json".to_string(),
        byte_count: 2,
        data_base64: None,
        text: Some("[]".to_string()),
        approved_file_receipt: None,
    }];
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "heuristic_filter".to_string(),
        reason: "local read".to_string(),
        matched_signals: Vec::new(),
        status_label: "OOMU is planning local actions...".to_string(),
    };

    let filtered = filter_conversational_mcp_tool_capabilities_for_turn(
        &capabilities,
        &attachments,
        &decision,
        "What is on my calendar today?",
    );
    let tool_names = filtered
        .iter()
        .map(|capability| capability.tool_name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(tool_names, vec!["read_system_calendar", "read_file"]);
    let context = workspace_data_attachment_context(&attachments).expect("mail context directive");
    assert!(context.contains(crate::agent_manager::WORKSPACE_DATA_ATTACHMENT_PRIORITY_DIRECTIVE));
    assert!(context.contains("emails"));
}

#[test]
fn technical_prompt_does_not_remove_connected_productivity_tools() {
    let capabilities = vec![
        conversational_capability("macos_applescript", "read_system_emails"),
        conversational_capability("macos_applescript", "read_system_calendar"),
        conversational_capability("macos_applescript", "read_system_reminders"),
        conversational_capability("local_filesystem", "read_file"),
    ];
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "heuristic_filter".to_string(),
        reason: "local read".to_string(),
        matched_signals: Vec::new(),
        status_label: "OOMU is planning local actions...".to_string(),
    };

    let filtered = filter_conversational_mcp_tool_capabilities_for_turn(
        &capabilities,
        &[],
        &decision,
        "Schedule asynchronous data packets across a mesh network.",
    );
    let tool_names = filtered
        .iter()
        .map(|capability| capability.tool_name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        tool_names,
        vec![
            "read_system_emails",
            "read_system_calendar",
            "read_system_reminders",
            "read_file",
        ]
    );
}

#[test]
fn non_english_prompts_keep_connected_tool_schemas_available() {
    let capabilities = vec![
        conversational_capability("macos_applescript", "read_system_calendar"),
        conversational_capability("macos_applescript", "read_system_emails"),
        conversational_capability("local_filesystem", "read_file"),
    ];
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
        requires_local_access: false,
        decision_source: "deterministic_action_rules".to_string(),
        reason: "no English action phrase matched".to_string(),
        matched_signals: Vec::new(),
        status_label: "OOMU is typing...".to_string(),
    };

    for prompt in [
        "Accede a mi calendario y dime qué tengo hoy.",
        "今日のカレンダーを確認して。",
        "Перевір мій календар на сьогодні.",
    ] {
        let filtered = filter_conversational_mcp_tool_capabilities_for_turn(
            &capabilities,
            &[],
            &decision,
            prompt,
        );
        assert_eq!(
            conversational_capability_keys(&filtered),
            conversational_capability_keys(&capabilities),
            "prompt unexpectedly removed connected tools: {prompt}"
        );
    }
}

#[test]
fn conversational_mcp_contract_does_not_truncate_connected_catalog_at_sixteen() {
    let capabilities = (0..20)
        .map(|index| ConversationalMcpToolCapability {
            server_name: "connected_tools".to_string(),
            tool_name: format!("tool_{index:02}"),
            description: format!("Connected tool {index}"),
            input_schema: serde_json::json!({"type":"object"}),
        })
        .collect::<Vec<_>>();

    let contract = conversational_mcp_tool_contract(&capabilities)
        .expect("the connected catalog should produce a model contract");
    for capability in &capabilities {
        assert!(
            contract.contains(&format!(
                "{}/{}",
                capability.server_name, capability.tool_name
            )),
            "contract omitted {}",
            capability.tool_name
        );
    }
}

#[test]
fn agent_identity_core_block_includes_host_hardware_metadata() {
    let identity_context = AgentIdentityContext {
        agent_id: "agent-oomu".to_string(),
        soul: crate::memory_ledger::AgentSoulManifest {
            agent_id: "agent-oomu".to_string(),
            display_name: "OOMU".to_string(),
            origin_story: "Local runtime agent.".to_string(),
            role: "Workstation AI".to_string(),
            values: vec!["Capability transparency".to_string()],
            hard_boundaries: vec!["Do not invent hardware facts.".to_string()],
            communication_style: "Precise".to_string(),
            self_description: "Hardware-aware local assistant.".to_string(),
            immutable_truths: vec!["Runs through OOMU.".to_string()],
            version: 1,
            signature: crate::sovereign_identity::SignatureBlock::default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        },
        memories: Vec::new(),
        user_profile: None,
        path_context: None,
        prompt_context: String::new(),
        secure_memory_available: true,
    };

    let prompt = format_agent_identity_core_block(&identity_context, "local_model", "gemma-4-2b");

    assert!(prompt.contains("[HOST HARDWARE METADATA]"));
    assert!(prompt.contains("- CPU Architecture:"));
    assert!(prompt.contains("- Logical CPU Cores:"));
    assert!(prompt.contains("- Physical RAM:"));
    assert!(prompt.contains("- Metal Backend Available:"));
    assert!(!prompt.contains("Estimated VRAM"));
    assert!(!prompt.contains("System Compute Score"));
    assert!(prompt.contains("Use host hardware metadata"));
}

#[test]
fn lean_local_identity_keeps_persona_boundaries_without_hardware_noise() {
    let identity_context = AgentIdentityContext {
        agent_id: "agent-oomu".to_string(),
        soul: crate::memory_ledger::AgentSoulManifest {
            agent_id: "agent-oomu".to_string(),
            display_name: "OOMU".to_string(),
            origin_story: "OOMU resident strategist.".to_string(),
            role: "Architect and Strategist".to_string(),
            values: vec!["Clarity".to_string()],
            hard_boundaries: vec!["Never invent facts.".to_string()],
            communication_style: "Warm and incisive".to_string(),
            self_description: "A practical strategic partner.".to_string(),
            immutable_truths: vec!["Runs through OOMU.".to_string()],
            version: 1,
            signature: crate::sovereign_identity::SignatureBlock::default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        },
        memories: Vec::new(),
        user_profile: None,
        path_context: None,
        prompt_context: String::new(),
        secure_memory_available: true,
    };

    let prompt =
        format_lean_agent_identity_core_block(&identity_context, "local_model", "gemma-4-12b");

    assert!(prompt.contains("Name: OOMU"));
    assert!(prompt.contains("Role: Architect and Strategist"));
    assert!(prompt.contains("Never invent facts."));
    assert!(prompt.contains("active mod contract"));
    assert!(prompt.contains("Recent messages supplied with this request"));
    assert!(!prompt.contains("[HOST HARDWARE METADATA]"));
    assert!(!prompt.contains("CPU Architecture"));
}

#[test]
fn lean_local_persona_contains_only_conversation_essentials() {
    let mut profile = AgentPersonalityProfile::default();
    profile.identity.display_name = "OOMU".to_string();
    profile.identity.role = "Architect and Strategist".to_string();
    profile.personality.summary = "A practical strategic partner.".to_string();
    profile.personality.traits = vec!["warm".to_string(), "incisive".to_string()];
    profile.personality.tone = "Calm and direct".to_string();
    profile.relationship.user_address = "Alex".to_string();
    profile.relationship.boundaries = vec!["Never manipulate the user.".to_string()];

    let prompt = format_lean_local_persona_prompt(&profile);

    assert!(prompt.contains("Name: OOMU"));
    assert!(prompt.contains("Role: Architect and Strategist"));
    assert!(prompt.contains("- warm"));
    assert!(prompt.contains("Address the user as: Alex"));
    assert!(prompt.contains("Never manipulate the user."));
    assert!(!prompt.contains("Template ID"));
    assert!(!prompt.contains("Strategic playbook"));
    assert!(!prompt.contains("tool registry"));
}

#[test]
fn lean_context_is_limited_to_plain_local_conversation() {
    let routine = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
        requires_local_access: false,
        decision_source: "deterministic_action_rules".to_string(),
        reason: "No action rule matched.".to_string(),
        matched_signals: Vec::new(),
        status_label: "OOMU is typing...".to_string(),
    };
    assert!(should_use_lean_local_chat_context(
        true, &routine, false, false, false, false
    ));
    assert!(!should_use_lean_local_chat_context(
        true, &routine, true, false, false, false
    ));
    assert!(!should_use_lean_local_chat_context(
        true, &routine, false, true, false, false
    ));
    assert!(!should_use_lean_local_chat_context(
        true, &routine, false, false, true, false
    ));
    assert!(!should_use_lean_local_chat_context(
        true, &routine, false, false, false, true
    ));

    let mut memory_workflow = routine.clone();
    memory_workflow.decision_source = "internal_memory_profile_filter".to_string();
    memory_workflow.matched_signals = vec!["internal_memory_profile".to_string()];
    assert!(!should_use_lean_local_chat_context(
        true,
        &memory_workflow,
        false,
        false,
        false,
        false
    ));
}

#[test]
fn lean_long_term_context_keeps_fresh_memory_and_mod_knowledge() {
    let identity = identity_context_with_memories(vec![crate::memory_ledger::AgentMemoryEntry {
        id: 1,
        agent_id: "agent-memory-test".to_string(),
        memory_kind: "daily_journal".to_string(),
        scope: "journal:2026-06-03".to_string(),
        content: "Journal date: 2026-06-03\nSource file: 2026-06-03.md\nEntry: Blue, owned by Omar"
            .to_string(),
        confidence: 0.86,
        source_session: "journal_import:2026-06-03.md".to_string(),
        source_turn: None,
        contradicted_by: None,
        visibility: "private".to_string(),
        signature: crate::sovereign_identity::SignatureBlock::default(),
        created_at_ms: 0,
        last_confirmed_at_ms: None,
    }]);
    let blocks = build_lean_chat_long_term_blocks(&identity, Some("verified active mod reference"));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].label, "Dynamic Durable Memory Matches");
    assert!(blocks[0].content.contains("2026-06-03.md"));
    assert!(blocks[0].content.contains("Blue, owned by Omar"));
    assert!(blocks[0]
        .content
        .contains("conflict with an earlier chat answer"));
    assert_eq!(blocks[1].label, "Isolated Mod Knowledge Retrieval");
    assert_eq!(blocks[1].content, "verified active mod reference");
}

#[test]
fn referential_follow_up_promotes_verified_recent_quote() {
    let messages = vec![
        test_message(
            "user",
            "All right, all right, all right. I've got to tell you something.",
        ),
        test_message("assistant", "Go ahead, Alex."),
        test_message("user", "What movie is that from and who said it?"),
    ];

    let block =
        verified_recent_referent_block("What movie is that from and who said it?", &messages)
            .expect("referential question should promote recent raw turns");

    assert_eq!(
        block.label,
        "Verified Recent Conversation Reference (High Priority)"
    );
    assert!(block.content.contains("All right, all right, all right."));
    assert!(block.content.contains("User:"));
    assert!(block.content.contains("Assistant:"));
    assert!(!block
        .content
        .contains("What movie is that from and who said it?"));

    let assembly = context_manager::assemble_context(ContextAssemblyRequest {
        static_core_blocks: Vec::new(),
        working_context_blocks: vec![block],
        working_messages: vec![test_message(
            "user",
            "What movie is that from and who said it?",
        )],
        long_term_blocks: Vec::new(),
        token_budget: Some(512),
        working_turn_limit: 1,
    });
    assert!(assembly
        .system_prompt
        .contains("All right, all right, all right."));
}

#[test]
fn referential_detector_rejects_standalone_and_dummy_pronoun_questions() {
    for standalone in [
        "What movie should I watch?",
        "How does it rain?",
        "Why is it raining?",
        "What is the weather tomorrow?",
        "What is Italy known for?",
        "Where is Italy?",
        "What is iteration in programming?",
        "How does the itemized deduction work?",
        "Why is there inflation?",
        "How is there still a shortage?",
    ] {
        assert!(
            !is_referential_follow_up(standalone),
            "standalone question must not pull prior chat: {standalone}"
        );
    }

    for follow_up in [
        "What movie and who said that?",
        "Can you explain that?",
        "Who is he?",
        "What did I say earlier?",
    ] {
        assert!(
            is_referential_follow_up(follow_up),
            "genuine backward reference should be detected: {follow_up}"
        );
    }
}

#[test]
fn referent_context_keeps_six_complete_pairs_newest_first_with_bounded_prefix() {
    let mut messages = Vec::new();
    for index in 0..7 {
        messages.push(test_message(
            "user",
            &format!("user turn {index} {}", "u".repeat(700)),
        ));
        messages.push(test_message(
            "assistant",
            &format!("assistant turn {index} {}", "a".repeat(700)),
        ));
    }
    messages.push(test_message("user", "What did that refer to?"));

    let block = verified_recent_referent_block("What did that refer to?", &messages)
        .expect("deictic follow-up should build verified recent context");
    let immediate = block
        .content
        .find("Immediate antecedent")
        .expect("immediate antecedent marker");
    let pair_one = block.content.find("Pair 1").expect("newest pair marker");
    let pair_two = block
        .content
        .find("Pair 2")
        .expect("second-newest pair marker");

    assert!(immediate < pair_one && pair_one < pair_two);
    assert!(block.content[..1_500].contains("assistant turn 6"));
    assert!(block.content.contains("user turn 6"));
    assert!(block.content.contains("assistant turn 1"));
    assert!(!block.content.contains("user turn 0"));
    assert_eq!(block.content.matches("Pair ").count(), 6);
    assert!(block.content.chars().count() <= 6_503);
}

#[test]
fn verified_referential_turns_buffer_stream_until_response_validation() {
    assert!(should_buffer_referential_response(
        "What movie and who said that?",
        true
    ));
    assert!(!should_buffer_referential_response(
        "What movie and who said that?",
        false
    ));
    assert!(!should_buffer_referential_response(
        "What movie should I watch?",
        true
    ));
}

#[test]
fn factual_grounding_turns_buffer_stream_until_headless_claim_validation() {
    assert!(should_buffer_validation_sensitive_response(
        "/research compare public options",
        false,
        true,
        false,
    ));
    assert!(should_buffer_validation_sensitive_response(
        "What’s the latest edition of the book Writing AI Prompts for Dummies?",
        false,
        false,
        true,
    ));
    assert!(!should_buffer_validation_sensitive_response(
        "Recommend three novels",
        false,
        false,
        false,
    ));
}

#[test]
fn validation_gate_retains_the_accepted_response_stream_owner() {
    let (provider, accepted) = validated_stream::split_handles(Some("stream"), true);
    assert_eq!(provider, Some("stream"));
    assert_eq!(accepted, Some("stream"));

    let (provider, accepted) = validated_stream::split_handles(Some("stream"), false);
    assert_eq!(provider, Some("stream"));
    assert_eq!(accepted, Some("stream"));

    let (provider, accepted) = validated_stream::split_handles::<&str>(None, true);
    assert_eq!((provider, accepted), (None, None));
}

#[test]
fn ordinary_new_question_does_not_duplicate_recent_dialogue() {
    let messages = vec![
        test_message("user", "A prior unrelated subject."),
        test_message("assistant", "A prior answer."),
        test_message("user", "Recommend three science fiction novels."),
    ];

    assert!(
        verified_recent_referent_block("Recommend three science fiction novels.", &messages,)
            .is_none()
    );
}

#[test]
fn compaction_checkpoint_becomes_working_context_not_provider_dialogue() {
    let mut messages = vec![
            test_message(
                "system",
                "Compacted conversation excerpts. Every entry below is verified.\n[source role=user] Earlier quote.",
            ),
            test_message("user", "What did I say earlier?"),
        ];

    let blocks = take_compaction_checkpoint_blocks(&mut messages);

    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].content.contains("Earlier quote."));
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
    assert!(messages
        .iter()
        .all(|message| !message.role.eq_ignore_ascii_case("system")));
}

#[test]
fn safeguarded_context_budget_caps_standard_chat() {
    assert_eq!(
        compile_safeguarded_context_budget(1_000_000, false, false),
        STANDARD_CHAT_CONTEXT_CAP_TOKENS
    );
    assert_eq!(
        compile_safeguarded_context_budget(8_192, false, false),
        8_192
    );
    assert_eq!(compile_safeguarded_context_budget(128, false, false), 512);

    let capped_allocation =
        jit_context_allocation(compile_safeguarded_context_budget(1_000_000, false, false));
    assert_eq!(
        capped_allocation.history_message_limit,
        STANDARD_CHAT_CONTEXT_CAP_TOKENS / JIT_AVERAGE_MESSAGE_TOKENS
    );
}

#[test]
fn safeguarded_context_budget_opens_grounding_bypass_valve() {
    assert_eq!(
        compile_safeguarded_context_budget(128_000, true, false),
        128_000
    );
    assert_eq!(
        compile_safeguarded_context_budget(1_000_000, false, true),
        1_000_000
    );
    assert_eq!(
        compile_safeguarded_context_budget(2_000_000, true, true),
        MAX_GROUNDED_CONTEXT_BUDGET_TOKENS
    );
}

#[test]
fn disabled_web_grounding_does_not_open_grounding_bypass() {
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
        requires_local_access: false,
        decision_source: "web_grounding_disabled_filter".to_string(),
        reason: "Automated web grounding is disabled.".to_string(),
        matched_signals: vec!["web grounding disabled".to_string()],
        status_label: "OOMU is typing...".to_string(),
    };

    assert!(!route_has_explicit_grounding_context(None, &decision));
    assert_eq!(
        compile_safeguarded_context_budget(
            1_000_000,
            false,
            route_has_explicit_grounding_context(None, &decision),
        ),
        STANDARD_CHAT_CONTEXT_CAP_TOKENS
    );
}

#[test]
fn standard_chat_auto_compaction_preserves_pending_turn() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-sprint-164-compaction-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    for turn in 0..7 {
        engine
            .insert_chat_message(
                "session-a",
                "agent-a",
                "user",
                &format!("turn {turn} {}", "alpha ".repeat(1600)),
            )
            .unwrap();
        engine
            .insert_chat_message(
                "session-a",
                "agent-a",
                "assistant",
                &format!("turn {turn} {}", "beta ".repeat(1600)),
            )
            .unwrap();
    }

    let pending_turn = "Please continue from the compacted context.";
    let compacted =
        maybe_compact_standard_chat_history(&engine, "session-a", 512, false, Some(pending_turn))
            .unwrap();
    assert!(compacted);

    let active_after_compaction = engine.select_chat_messages("session-a").unwrap();
    assert_eq!(active_after_compaction.len(), 13);
    assert_eq!(
        active_after_compaction[0].compaction_type.as_deref(),
        Some("summary_anchor")
    );
    assert_eq!(
        active_after_compaction
            .iter()
            .filter(|message| message.role == "user")
            .count(),
        6
    );
    assert_eq!(
        active_after_compaction
            .iter()
            .filter(|message| message.role == "assistant")
            .count(),
        6
    );

    engine
        .insert_chat_message("session-a", "agent-a", "user", pending_turn)
        .unwrap();
    let history = engine.get_chat_history("session-a", 20).unwrap();
    assert_eq!(history.len(), 14);
    assert_eq!(history.last().unwrap().role, "user");
    assert_eq!(history.last().unwrap().content, pending_turn);

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn standard_chat_auto_compaction_honors_disabled_session_policy() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu-sprint-299-compaction-policy-{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    for turn in 0..7 {
        engine
            .insert_chat_message(
                "session-a",
                "agent-a",
                "user",
                &format!("turn {turn} {}", "alpha ".repeat(1600)),
            )
            .unwrap();
        engine
            .insert_chat_message(
                "session-a",
                "agent-a",
                "assistant",
                &format!("turn {turn} {}", "beta ".repeat(1600)),
            )
            .unwrap();
    }
    engine
        .save_session_context_policy(&crate::db::SaveSessionContextPolicyRequest {
            session_id: "session-a".to_string(),
            auto_compaction_threshold_percent: 70,
            auto_compaction_enabled: false,
        })
        .unwrap();

    let compacted = maybe_compact_standard_chat_history(
        &engine,
        "session-a",
        512,
        false,
        Some("Keep the pending turn intact."),
    )
    .unwrap();
    assert!(!compacted);
    assert_eq!(engine.select_chat_messages("session-a").unwrap().len(), 14);

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn steering_user_message_is_durable_but_absent_from_current_history_snapshot() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-steering-persistence-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    engine
        .insert_chat_message("session-a", "agent-a", "user", "Write the initial answer.")
        .unwrap();
    engine
        .insert_chat_message("session-a", "agent-a", "assistant", "Initial draft")
        .unwrap();

    let current_history_snapshot = engine.get_chat_history("session-a", 10).unwrap();
    let turn_context = ChatTurnPersistenceContext {
        turn_id: "turn-steer".to_string(),
        generation_token: "generation-steer".to_string(),
        session_id: "session-a".to_string(),
        agent_id: "agent-a".to_string(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-test".to_string(),
        parent_turn_id: Some("turn-root".to_string()),
        root_turn_id: "turn-root".to_string(),
        turn_kind: "steer".to_string(),
    };
    let metadata = serde_json::json!({
        "turnId": turn_context.turn_id.as_str(),
        "turnKind": turn_context.turn_kind.as_str(),
    });

    persist_steering_user_message(&engine, &turn_context, "Use Markdown headings.", &metadata)
        .unwrap();

    assert_eq!(current_history_snapshot.len(), 2);
    assert!(current_history_snapshot
        .iter()
        .all(|message| message.content != "Use Markdown headings."));

    let persisted = engine.select_chat_messages("session-a").unwrap();
    let steering_message = persisted.last().unwrap();
    assert_eq!(steering_message.role, "user");
    assert_eq!(steering_message.content, "Use Markdown headings.");
    assert_eq!(steering_message.provider_id.as_deref(), Some("local_model"));
    assert_eq!(steering_message.model_id.as_deref(), Some("gemma-test"));
    let persisted_metadata: Value =
        serde_json::from_str(steering_message.metadata_json.as_deref().unwrap()).unwrap();
    assert_eq!(persisted_metadata["turnKind"], "steer");

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn explicit_internal_memory_is_signed_before_inference_dispatch() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-pre-inference-memory-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let ledger = MemoryLedger::initialize_at(temp_dir.join("oomu_ops.sqlite")).unwrap();
    let identity = SovereignIdentity::initialize_ephemeral();

    let captured = capture_pre_inference_internal_memories(
        &ledger,
        "internal_memory_profile_filter",
        false,
        CaptureChatMemoriesRequest {
            agent_id: "oomu".to_string(),
            display_name: "OOMU".to_string(),
            role: "Workstation AI".to_string(),
            description: "Test agent".to_string(),
            session_id: "session-jeff".to_string(),
            user_message: "Yes, call me Alex and make note of that in your memories".to_string(),
            assistant_message: String::new(),
            project_id: None,
        },
        &identity,
    )
    .unwrap();
    assert!(!captured.is_empty());
    for memory in &captured {
        verify_agent_memory(memory, &identity).unwrap();
    }

    let hydrated = ledger
        .hydrate_agent_context_sync(
            HydrateAgentContextRequest {
                agent_id: "oomu".to_string(),
                display_name: "OOMU".to_string(),
                role: "Workstation AI".to_string(),
                description: "Test agent".to_string(),
                system_prompt: "Test".to_string(),
                latest_message: "How should you address me?".to_string(),
                provider_id: Some("local_model".to_string()),
                model_id: Some("gemma-test".to_string()),
                tool_registry_offline: true,
                background_mod_event: false,
                layout_schema: None,
                project_id: None,
                verified_filesystem_context: None,
            },
            &identity,
        )
        .unwrap();
    assert_eq!(
        hydrated
            .user_profile
            .as_ref()
            .map(|profile| profile.display_name.as_str()),
        Some("Alex")
    );

    drop(ledger);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn grounding_bypass_skips_standard_chat_auto_compaction() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu-sprint-164-grounding-bypass-{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    engine
        .insert_chat_message("session-a", "agent-a", "user", &"alpha ".repeat(1600))
        .unwrap();
    engine
        .insert_chat_message("session-a", "agent-a", "assistant", &"beta ".repeat(1600))
        .unwrap();

    let compacted = maybe_compact_standard_chat_history(
        &engine,
        "session-a",
        512,
        true,
        Some("Large grounded turn follows."),
    )
    .unwrap();
    assert!(!compacted);
    assert_eq!(engine.select_chat_messages("session-a").unwrap().len(), 2);

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

mod continuation;
mod continuation_runtime;
mod output_integrity;
mod provider_stream;
