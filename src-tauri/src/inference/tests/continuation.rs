use super::*;

#[test]
fn remote_request_does_not_claim_native_tools_without_provider_tool_schemas() {
    let prompt = "Be useful.";
    let normalized = normalize_request(InferenceRequest {
        provider_id: "gemini".to_string(),
        model_id: "gemini-test".to_string(),
        system_prompt: Some(prompt.to_string()),
        messages: Vec::new(),
        prompt: Some("Hello".to_string()),
        temperature: None,
        max_tokens: None,
        reasoning: None,
        reasoning_budget_tokens: None,
        base_url: None,
        api_key_label: None,
        api_key: None,
    })
    .expect("provider request normalizes");

    assert_eq!(normalized.system_prompt.as_deref(), Some(prompt));
    assert!(!normalized
        .system_prompt
        .as_deref()
        .unwrap_or_default()
        .contains("REGISTERED"));
    assert!(!normalized
        .system_prompt
        .as_deref()
        .unwrap_or_default()
        .contains("file_write"));
}

#[test]
fn profile_persistence_claim_detector_rejects_only_positive_completion_claims() {
    assert!(assistant_claims_profile_persistence(
        "I've updated your profile with that preference."
    ));
    assert!(assistant_claims_profile_persistence(
        "I'll remember that for future conversations."
    ));
    assert!(!assistant_claims_profile_persistence(
        "I cannot save that preference without a native receipt."
    ));
    assert!(!assistant_claims_profile_persistence(
        "I can use that preference for this response."
    ));
}

#[test]
fn profile_persistence_receipts_require_a_profile_memory_kind() {
    for memory_kind in ["user_profile", "relationship_notes", "agent_self"] {
        assert!(is_profile_persistence_memory_kind(memory_kind));
    }
    for memory_kind in ["project_context", "document", "tool_output"] {
        assert!(!is_profile_persistence_memory_kind(memory_kind));
    }
}

#[test]
fn sprint_150_classifies_transient_and_fatal_inference_errors() {
    let network_timeout = InferenceError::network("operation timed out while reading stream");
    assert_eq!(
        classify_inference_error(&network_timeout),
        InferenceFailureClass::Transient
    );

    let rate_limited = InferenceError::provider_rate_limited();
    assert_eq!(
        classify_inference_error(&rate_limited),
        InferenceFailureClass::Transient
    );

    let interrupted = InferenceError::provider_stream_interrupted_after_tokens(
        "The provider connection closed before the response finished.",
    );
    assert_eq!(
        classify_inference_error(&interrupted),
        InferenceFailureClass::Transient
    );
    assert!(retry_allowed_for_stream_state(&interrupted, Some(142)));
    let exhausted_interruption = InferenceError::retry_exhausted(&interrupted, 3);
    assert!(should_attempt_failover(&exhausted_interruption, false));
    assert!(!should_attempt_failover(&exhausted_interruption, true));

    let unterminated = provider_stream_ended_before_terminal_error(142);
    assert_eq!(
        unterminated.code,
        "provider_stream_interrupted_after_tokens"
    );
    assert_eq!(
        classify_inference_error(&unterminated),
        InferenceFailureClass::Transient
    );

    let duration_exceeded = provider_stream_duration_exceeded_error();
    assert_eq!(duration_exceeded.code, "provider_stream_duration_exceeded");
    assert_eq!(
        classify_inference_error(&duration_exceeded),
        InferenceFailureClass::Fatal
    );

    let metal_pause = InferenceError::local_infer(
        "llama_context_init_failed",
        "Metal context compilation pause stalled local generation.",
    );
    assert_eq!(
        classify_inference_error(&metal_pause),
        InferenceFailureClass::Transient
    );

    let auth = InferenceError::credential("invalid API token");
    assert_eq!(
        classify_inference_error(&auth),
        InferenceFailureClass::Fatal
    );

    let unauthorized =
        InferenceError::network("Provider HTTP request failed with status 401 Unauthorized.");
    assert_eq!(
        classify_inference_error(&unauthorized),
        InferenceFailureClass::Fatal
    );

    let invalid = InferenceError::invalid("temperature parameter is out of bounds");
    assert_eq!(
        classify_inference_error(&invalid),
        InferenceFailureClass::Fatal
    );

    let schema = InferenceError::provider("Provider payload failed schema validation.");
    assert_eq!(
        classify_inference_error(&schema),
        InferenceFailureClass::Fatal
    );

    let cancelled = InferenceError::local_infer(
        "local_inference_cancelled",
        "Generation was cancelled by the user.",
    );
    assert_eq!(
        classify_inference_error(&cancelled),
        InferenceFailureClass::Fatal
    );
}

#[test]
fn gemini_zero_text_malformed_function_call_is_retryable_and_failover_eligible() {
    let error = InferenceError::provider(
        "Google Gemini stopped with MALFORMED_FUNCTION_CALL before returning visible text.",
    );
    assert_eq!(
        classify_inference_error(&error),
        InferenceFailureClass::Transient
    );
    assert!(should_attempt_failover(&error, false));

    let exhausted = InferenceError::retry_exhausted(&error, 3);
    assert!(should_attempt_failover(&exhausted, false));
}

#[test]
fn sprint_150_transient_retry_succeeds_on_second_attempt() {
    let attempts = std::cell::Cell::new(0usize);
    let delays = std::cell::RefCell::new(Vec::new());

    let result = execute_with_transient_inference_retry_and_sleep(
        "test_transient_success",
        || {
            attempts.set(attempts.get() + 1);
            if attempts.get() == 1 {
                Err(InferenceError::network("operation timed out"))
            } else {
                Ok("recovered")
            }
        },
        |_| true,
        |delay| delays.borrow_mut().push(delay),
    )
    .expect("second attempt recovers");

    assert_eq!(result, "recovered");
    assert_eq!(attempts.get(), 2);
    assert_eq!(delays.borrow().as_slice(), &[Duration::from_millis(1_000)]);
}

#[test]
fn sprint_150_transient_retry_exhausts_after_three_attempts() {
    let attempts = std::cell::Cell::new(0usize);
    let delays = std::cell::RefCell::new(Vec::new());

    let error = execute_with_transient_inference_retry_and_sleep::<(), _, _, _>(
        "test_transient_exhaustion",
        || {
            attempts.set(attempts.get() + 1);
            Err(InferenceError::provider_rate_limited())
        },
        |_| true,
        |delay| delays.borrow_mut().push(delay),
    )
    .expect_err("third transient failure exhausts retries");

    assert_eq!(attempts.get(), TRANSIENT_INFERENCE_MAX_ATTEMPTS);
    assert_eq!(
        delays.borrow().as_slice(),
        &[Duration::from_millis(1_000), Duration::from_millis(2_000)]
    );
    assert_eq!(error.code, "inference_retry_exhausted");
    assert!(error.message.contains("provider_rate_limited"));
    assert!(!error.message.contains("HTTP 429"));
    assert!(!error.message.contains("message="));
    assert_eq!(
        classify_inference_error(&error),
        InferenceFailureClass::Fatal
    );
}

#[test]
fn provider_retry_diagnostics_are_redacted_and_bounded_before_logging() {
    let detail = format!(
        "request failed: https://example.test/path?key=secret-canary&token=token-canary {}",
        "x".repeat(MAX_PROVIDER_ERROR_LOG_CHARS * 2),
    );
    let bounded = bounded_provider_error_log_detail(&detail);

    assert!(!bounded.contains("secret-canary"));
    assert!(!bounded.contains("token-canary"));
    assert!(bounded.contains("key=[redacted]"));
    assert!(bounded.chars().count() <= MAX_PROVIDER_ERROR_LOG_CHARS + 1);
}

#[test]
fn sprint_150_fatal_inference_errors_bypass_retry() {
    let attempts = std::cell::Cell::new(0usize);
    let delays = std::cell::RefCell::new(Vec::new());

    let error = execute_with_transient_inference_retry_and_sleep::<(), _, _, _>(
        "test_fatal_bypass",
        || {
            attempts.set(attempts.get() + 1);
            Err(InferenceError::credential("invalid API token"))
        },
        |_| true,
        |delay| delays.borrow_mut().push(delay),
    )
    .expect_err("fatal auth error bypasses retry");

    assert_eq!(attempts.get(), 1);
    assert!(delays.borrow().is_empty());
    assert_eq!(error.code, "credential_unavailable");
}

#[test]
fn local_inference_timeouts_do_not_repeat_the_expensive_local_attempt() {
    for code in ["local_inference_startup_timeout", "local_inference_timeout"] {
        let attempts = std::cell::Cell::new(0usize);
        let delays = std::cell::RefCell::new(Vec::new());

        let error = execute_with_transient_inference_retry_and_sleep::<(), _, _, _>(
            "test_local_timeout_bypass",
            || {
                attempts.set(attempts.get() + 1);
                Err(InferenceError::local_infer(code, "bounded local timeout"))
            },
            |_| true,
            |delay| delays.borrow_mut().push(delay),
        )
        .expect_err("a local timeout must be returned after one attempt");

        assert_eq!(attempts.get(), 1, "timeout code {code} retried");
        assert!(delays.borrow().is_empty());
        assert_eq!(error.code, code);
        assert_eq!(
            classify_inference_error(&error),
            InferenceFailureClass::Fatal
        );
    }
}

#[test]
fn backend_executable_gate_prevents_pivot_only_false_escalations() {
    let stale_escalation = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "stale_preflight".to_string(),
        reason: "file extension: .9".to_string(),
        matched_signals: vec!["file extension: .9".to_string()],
        status_label: "Planning".to_string(),
    };
    let conversational = enforce_backend_executable_intent_gate(
        stale_escalation.clone(),
        "99.9% of users will not know that.",
        &[],
    );
    assert!(matches!(
        conversational.route,
        crate::agentic_loop::ChatIntentRoute::ConversationalStream
    ));
    assert_eq!(
        conversational.decision_source,
        "backend_executable_intent_gate"
    );

    let executable =
        enforce_backend_executable_intent_gate(stale_escalation, "Delete report.pdf.", &[]);
    assert!(matches!(
        executable.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ));
}

#[test]
fn backend_executable_gate_preserves_unhydrated_private_app_reads() {
    let planner_decision = || crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "private_app_data_filter".to_string(),
        reason: "Mail requires a protected native read.".to_string(),
        matched_signals: vec!["private mail request".to_string()],
        status_label: "Checking Mail".to_string(),
    };

    let unread_mail = enforce_backend_executable_intent_gate(
        planner_decision(),
        "Do I have any unread emails?",
        &[],
    );
    assert!(matches!(
        unread_mail.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ));
    assert!(unread_mail.requires_local_access);
    assert_eq!(unread_mail.decision_source, "private_app_data_filter");

    let informational = enforce_backend_executable_intent_gate(
        planner_decision(),
        "How does Apple Mail organize unread emails?",
        &[],
    );
    assert!(matches!(
        informational.route,
        crate::agentic_loop::ChatIntentRoute::ConversationalStream
    ));
    assert!(!informational.requires_local_access);
    assert_eq!(
        informational.decision_source,
        "backend_executable_intent_gate"
    );

    let impersonal_count = enforce_backend_executable_intent_gate(
        planner_decision(),
        "How many unread emails are normal?",
        &[],
    );
    assert!(matches!(
        impersonal_count.route,
        crate::agentic_loop::ChatIntentRoute::ConversationalStream
    ));
    assert!(!impersonal_count.requires_local_access);
    assert_eq!(
        impersonal_count.decision_source,
        "backend_executable_intent_gate"
    );

    let mutation = enforce_backend_executable_intent_gate(
        planner_decision(),
        "Compose an email in Mail and save it as a draft.",
        &[],
    );
    assert!(matches!(
        mutation.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ));
    assert!(mutation.requires_local_access);
}

#[test]
fn any_connected_catalog_prevents_phrase_based_agentic_escalation() {
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "private_app_data_filter".to_string(),
        reason: "Mail requires a protected native read.".to_string(),
        matched_signals: vec!["private mail request".to_string()],
        status_label: "Checking Mail".to_string(),
    };
    let mail_capability = ConversationalMcpToolCapability {
        server_name: "macos_applescript".to_string(),
        tool_name: "read_system_emails".to_string(),
        description: String::new(),
        input_schema: serde_json::json!({}),
    };

    assert!(!executable_intent_gate::requires_agentic_escalation(
        &decision,
        "Do I have any unread emails?",
        std::slice::from_ref(&mail_capability),
    ));
    assert!(executable_intent_gate::requires_agentic_escalation(
        &decision,
        "Do I have any unread emails?",
        &[],
    ));
    assert!(!executable_intent_gate::requires_agentic_escalation(
        &decision,
        "How many unread emails are normal?",
        std::slice::from_ref(&mail_capability),
    ));
    assert!(!executable_intent_gate::requires_agentic_escalation(
        &decision,
        "Do I have any unread emails? Then run npm test.",
        std::slice::from_ref(&mail_capability),
    ));
    assert!(!executable_intent_gate::requires_agentic_escalation(
        &decision,
        "Do I have any unread emails? Then flag them.",
        &[mail_capability],
    ));

    let arbitrary_connected_capability = ConversationalMcpToolCapability {
        server_name: "connected_customer_service".to_string(),
        tool_name: "lookup_customer".to_string(),
        description: String::new(),
        input_schema: serde_json::json!({}),
    };
    for prompt in [
        "Accede a mi calendario.",
        "今日のカレンダーを確認して。",
        "Перевір мій календар.",
    ] {
        assert!(!executable_intent_gate::requires_agentic_escalation(
            &decision,
            prompt,
            std::slice::from_ref(&arbitrary_connected_capability),
        ));
    }
}

#[test]
fn queued_route_escalations_are_terminal_failures_not_completed_responses() {
    let response = ChatTurnResponse {
        text: "Pivoting to Agentic Planner.".to_string(),
        session_id: "session-1".to_string(),
        turn_id: "turn-1".to_string(),
        generation_token: "generation-1".to_string(),
        metadata: None,
        route_escalation: Some(crate::agentic_loop::ChatIntentRouteDecision {
            route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
            requires_local_access: true,
            decision_source: "private_app_data_filter".to_string(),
            reason: "Foreground native execution required.".to_string(),
            matched_signals: Vec::new(),
            status_label: "Checking Mail".to_string(),
        }),
    };

    let error = queued_execution::route_escalation_failure(&response)
        .expect("a route escalation must not be recorded as queue completion");
    assert!(error.contains("foreground"));
    assert!(error.contains("Open this chat"));
}

#[test]
fn backend_hydrated_contacts_gate_never_reenters_action_planning() {
    let stale_escalation = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "private_app_data_filter".to_string(),
        reason: "Contacts require a protected native read.".to_string(),
        matched_signals: vec!["private contacts request".to_string()],
        status_label: "Checking Contacts".to_string(),
    };
    let attachments = vec![ChatAttachment {
        name: "local_contacts.json".to_string(),
        mime_type: "application/json".to_string(),
        byte_count: 192,
        data_base64: None,
        text: Some(
            "Local Contacts context\nSource: native_contacts/read_system_contacts\n[{\"displayName\":\"Maya Allan\"}]"
                .to_string(),
        ),
        approved_file_receipt: None,
    }];

    let conversational = enforce_backend_executable_intent_gate(
        stale_escalation,
        "Search my contacts and see if you can find Maya Allan",
        &attachments,
    );

    assert!(matches!(
        conversational.route,
        crate::agentic_loop::ChatIntentRoute::ConversationalStream
    ));
    assert!(!conversational.requires_local_access);
    assert_eq!(
        conversational.decision_source,
        "backend_hydrated_workspace_data_gate"
    );
}

#[test]
fn backend_hydrated_file_gate_never_reenters_action_planning() {
    let planner_decision = || crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "stale_file_route".to_string(),
        reason: "A file operation was requested.".to_string(),
        matched_signals: vec!["typed filename".to_string()],
        status_label: "Planning".to_string(),
    };
    let attachments = vec![ChatAttachment {
        name: "Screenshot 2026-07-13 at 21.39.23.png".to_string(),
        mime_type: "text/plain".to_string(),
        byte_count: 96,
        data_base64: None,
        text: Some("Visual analysis for the approved screenshot.".to_string()),
        approved_file_receipt: None,
    }];
    let prompt = "Can you view this file? '[approved file: Screenshot 2026-07-13 at 21.39.23.png]'";

    assert!(has_matching_approved_file_attachment(prompt, &attachments));

    let conversational =
        enforce_backend_executable_intent_gate(planner_decision(), prompt, &attachments);

    assert!(matches!(
        conversational.route,
        crate::agentic_loop::ChatIntentRoute::ConversationalStream
    ));
    assert!(!conversational.requires_local_access);
    assert_eq!(
        conversational.decision_source,
        "backend_hydrated_local_context_gate"
    );

    let mutation = enforce_backend_executable_intent_gate(
        planner_decision(),
        "View [approved file: Screenshot 2026-07-13 at 21.39.23.png] and then delete it.",
        &attachments,
    );
    assert!(matches!(
        mutation.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ));

    let unrelated = vec![ChatAttachment {
        name: "different.txt".to_string(),
        mime_type: "text/plain".to_string(),
        byte_count: 8,
        data_base64: None,
        text: Some("different".to_string()),
        approved_file_receipt: None,
    }];
    assert!(!has_matching_approved_file_attachment(prompt, &unrelated));
    let missing_context =
        enforce_backend_executable_intent_gate(planner_decision(), prompt, &unrelated);
    assert!(matches!(
        missing_context.route,
        crate::agentic_loop::ChatIntentRoute::ConversationalStream
    ));
    assert_eq!(
        missing_context.decision_source,
        "approved_file_context_missing_filter"
    );
}

#[test]
fn verified_approved_file_context_bypasses_preflight_and_preserves_display_copy() {
    let safe_prompt =
        "Can you view this file? '[approved file: Screenshot 2026-07-13 at 21.39.23.png]'";
    let display_prompt =
        "Can you view this file? '/Users/example/Desktop/Screenshot 2026-07-13 at 21.39.23.png'";
    let attachments = vec![ChatAttachment {
        name: "Screenshot 2026-07-13 at 21.39.23.png".to_string(),
        mime_type: "text/plain".to_string(),
        byte_count: 96,
        data_base64: None,
        text: Some("Visual analysis for the approved screenshot.".to_string()),
        approved_file_receipt: None,
    }];

    let route = project_chat::verified_route(false, safe_prompt, &attachments, true)
        .expect("verified bounded context must bypass preflight");
    assert!(matches!(
        route.route,
        crate::agentic_loop::ChatIntentRoute::ConversationalStream
    ));
    assert!(!route.requires_local_access);
    assert_eq!(route.decision_source, "verified_approved_file_context");
    assert_eq!(
        persisted_chat_user_content(safe_prompt, Some(display_prompt)),
        display_prompt
    );

    assert!(project_chat::verified_route(
        false,
        "View [approved file: Screenshot 2026-07-13 at 21.39.23.png] and delete it.",
        &attachments,
        true,
    )
    .is_none());
}

#[test]
fn claimed_chat_turn_guard_cannot_leave_a_running_generation() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_chat_turn_guard_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-guard".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-guard".to_string(),
            title: Some("Guard test".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-guard".to_string(),
        generation_token: "generation-guard".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: session.provider_id,
        model_id: session.model_id,
        parent_turn_id: None,
        root_turn_id: "turn-guard".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.begin_or_claim_chat_turn_response(&context).unwrap();
    {
        let _guard = ChatTurnPersistenceGuard::new(engine.clone(), context.clone(), false);
    }
    let status: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT status FROM chat_turns WHERE turn_id = ?1",
            rusqlite::params![context.turn_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "failed");
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn claimed_chat_turn_guard_persists_the_specific_terminal_error_code() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_chat_turn_failure_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-failure".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-failure".to_string(),
            title: Some("Failure test".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-failure".to_string(),
        generation_token: "generation-failure".to_string(),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        parent_turn_id: None,
        root_turn_id: "turn-failure".to_string(),
        turn_kind: "root".to_string(),
    };
    engine
        .accept_chat_turn(crate::db::AcceptChatTurnRequest {
            turn_id: context.turn_id.clone(),
            generation_token: context.generation_token.clone(),
            parent_turn_id: None,
            root_turn_id: context.root_turn_id.clone(),
            turn_kind: context.turn_kind.clone(),
            session_id: context.session_id.clone(),
            agent_id: context.agent_id.clone(),
            provider_id: context.provider_id.clone(),
            model_id: context.model_id.clone(),
            message: "Review the approved plan.".to_string(),
        })
        .unwrap();
    engine.begin_or_claim_chat_turn_response(&context).unwrap();
    let error = InferenceError {
        code: "workspace_boundary_violation".to_string(),
        boundary: "cognitive_isolation".to_string(),
        message: "internal detail must not become persisted display copy".to_string(),
    };
    let mut guard = ChatTurnPersistenceGuard::new(engine.clone(), context, false);
    guard.finish_inference_error(&error).unwrap();

    let messages = engine.select_chat_messages(&session.id).unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages[0]
        .metadata_json
        .as_deref()
        .unwrap()
        .contains("\"turnState\":\"failed\""));
    assert_eq!(
        messages[1].content,
        "OOMU couldn't finish this reply. Nothing was changed. Try again."
    );
    let metadata: serde_json::Value =
        serde_json::from_str(messages[1].metadata_json.as_deref().unwrap()).unwrap();
    assert_eq!(
        metadata["terminalErrorCode"],
        "workspace_boundary_violation"
    );
    assert_eq!(metadata["terminalErrorBoundary"], "cognitive_isolation");
    assert!(!messages[1].content.contains("internal detail"));
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn backend_hydrated_private_app_gate_matches_each_requested_resource() {
    let cases = [
        (
            "Check my calendar and tell me what I have today",
            "local_calendar.json",
            "Local Calendar context",
        ),
        (
            "Search my mail for a message from Maya",
            "local_mail.json",
            "Local Mail context",
        ),
        (
            "Check my reminders and tell me what is due",
            "local_reminders.json",
            "Local Reminders context",
        ),
        (
            "Search my notes for the project brief",
            "local_notes.json",
            "Local Notes context",
        ),
        (
            "Search my contacts and see if you can find Maya Allan",
            "local_contacts.json",
            "Local Contacts context",
        ),
        (
            "Show my newest photo",
            "local_photos.json",
            "Local Photos context",
        ),
        (
            "Show my recently added Apple Music songs",
            "local_music.json",
            "Local Music context",
        ),
        (
            "Show my recent Apple Messages conversations",
            "local_messages_ui.json",
            "Source: macos_applescript/read_apple_app_ui",
        ),
    ];

    for (prompt, attachment_name, attachment_text) in cases {
        let decision = crate::agentic_loop::ChatIntentRouteDecision {
            route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
            requires_local_access: true,
            decision_source: "stale_private_app_route".to_string(),
            reason: "A native read was requested.".to_string(),
            matched_signals: vec!["private app request".to_string()],
            status_label: "Checking app".to_string(),
        };
        let attachments = vec![ChatAttachment {
            name: attachment_name.to_string(),
            mime_type: "application/json".to_string(),
            byte_count: attachment_text.len(),
            data_base64: None,
            text: Some(attachment_text.to_string()),
            approved_file_receipt: None,
        }];

        let routed = enforce_backend_executable_intent_gate(decision, prompt, &attachments);

        assert!(
            matches!(
                routed.route,
                crate::agentic_loop::ChatIntentRoute::ConversationalStream
            ),
            "{prompt}"
        );
        assert_eq!(
            routed.decision_source, "backend_hydrated_workspace_data_gate",
            "{prompt}"
        );
    }
}

#[test]
fn backend_hydrated_private_app_gate_rejects_mismatches_and_mutations() {
    let planner_decision = || crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "stale_private_app_route".to_string(),
        reason: "A native operation was requested.".to_string(),
        matched_signals: vec!["private app request".to_string()],
        status_label: "Checking app".to_string(),
    };
    let contacts_attachment = vec![ChatAttachment {
        name: "local_contacts.json".to_string(),
        mime_type: "application/json".to_string(),
        byte_count: 2,
        data_base64: None,
        text: Some("[]".to_string()),
        approved_file_receipt: None,
    }];

    let unrelated = enforce_backend_executable_intent_gate(
        planner_decision(),
        "Check my calendar and tell me what I have today",
        &contacts_attachment,
    );
    assert!(matches!(
        unrelated.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ));

    let notes_attachment = vec![ChatAttachment {
        name: "local_notes.json".to_string(),
        mime_type: "application/json".to_string(),
        byte_count: 2,
        data_base64: None,
        text: Some("[]".to_string()),
        approved_file_receipt: None,
    }];
    let mutation = enforce_backend_executable_intent_gate(
        planner_decision(),
        "Create a note in my Apple Notes called Project Follow-up",
        &notes_attachment,
    );
    assert!(matches!(
        mutation.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ));
}

#[tokio::test]
async fn oversized_payload_still_runs_the_real_route_classifier() {
    let request = crate::agentic_loop::ChatIntentRouteRequest {
        prompt: "Audit this codebase.".to_string(),
        automated_web_grounding_enabled: None,
        attachments: vec![crate::agentic_loop::ChatIntentAttachment {
            name: "codebase.txt".to_string(),
            mime_type: "text/plain".to_string(),
            byte_count: 600_000,
            text: Some("fn main() {}\n".repeat(25_000)),
        }],
    };
    let policy = PreflightPolicy {
        timeout: Duration::from_secs(5),
    };

    let outcome = run_preflight_route_classification_with(request, policy, |_| async {
        Ok(crate::agentic_loop::ChatIntentRouteDecision {
            route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
            requires_local_access: true,
            decision_source: "test_real_classifier".to_string(),
            reason: "classifier executed".to_string(),
            matched_signals: vec!["explicit action".to_string()],
            status_label: "Planning".to_string(),
        })
    })
    .await
    .expect("oversized payload should still receive a real classification");

    assert!(matches!(
        outcome.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ));
    assert_eq!(outcome.decision_source, "test_real_classifier");
}

#[tokio::test]
async fn preflight_timeout_fails_closed_without_a_conversational_route() {
    let request = crate::agentic_loop::ChatIntentRouteRequest {
        prompt: "Review this bounded but complex local context.".to_string(),
        automated_web_grounding_enabled: None,
        attachments: Vec::new(),
    };
    let policy = PreflightPolicy {
        timeout: Duration::from_millis(10),
    };

    let error = run_preflight_route_classification_with(request, policy, |_| async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        unreachable!("timed-out classifier must be aborted")
    })
    .await
    .expect_err("timeout must not fabricate a conversational route");

    assert!(error.message.contains("no conversational bypass"));
}

#[test]
fn derived_turns_inherit_dynamic_parent_route_and_reject_concrete_route_changes() {
    assert!(private_auto_route::derived_route_request_is_compatible(
        Some("dynamic"),
        Some("dynamic"),
        "provider-resolved",
        "model-resolved",
    ));
    assert!(private_auto_route::derived_route_request_is_compatible(
        Some("provider-resolved"),
        Some("model-resolved"),
        "provider-resolved",
        "model-resolved",
    ));
    assert!(!private_auto_route::derived_route_request_is_compatible(
        Some("provider-other"),
        Some("model-resolved"),
        "provider-resolved",
        "model-resolved",
    ));
    assert!(!private_auto_route::derived_route_request_is_compatible(
        Some("provider-resolved"),
        Some("model-other"),
        "provider-resolved",
        "model-resolved",
    ));
}

#[test]
fn sprint_100_dynamic_response_metadata_records_actual_executor() {
    let terminal = local_usage::parse_terminal(
            r#"{"text":"Paris.","prompt_token_count":17,"generated_token_count":3,"device":"Metal","inference_latency_ms":25,"time_to_first_token_ms":20,"trace_hash":"trace"}"#,
        )
        .expect("local terminal parses");
    let response = InferenceResponse {
        provider_id: "local_model".to_string(),
        provider: "Local Model".to_string(),
        model_id: "gemma-4-12B-it-qat-q4_0-gguf".to_string(),
        text: "Paris.".to_string(),
        response_id: None,
        finish_reason: None,
        latency_ms: 25,
        local_usage: Some(local_usage::LocalInferenceUsage::from_terminal(
            "prompt", &terminal,
        )),
    };
    let dynamic_route = DynamicModelRouteDecision {
        local_provider_id: "local_model".to_string(),
        local_model_id: "gemma-4-12B-it-qat-q4_0-gguf".to_string(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-4-12B-it-qat-q4_0-gguf".to_string(),
        matched_complexity_rules: vec!["semantic:capability=general".to_string()],
        tier: "local_tier_1",
        reason: "test".to_string(),
        classifier_source: "local_difficulty_v2".to_string(),
        capability: "general".to_string(),
        demand: "routine".to_string(),
        confidence: "confident".to_string(),
        classification_reason: "bounded_transformation".to_string(),
        classifier_latency_ms: 12,
        classifier_model_id: Some("gemma-4-E2B-it-qat-q4_0-gguf".to_string()),
        readiness_generation: 4,
        recovery_attempted: false,
        policy_version: dynamic_routing::AUTO_ROUTE_POLICY_VERSION,
    };
    let route_decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
        requires_local_access: false,
        decision_source: "test".to_string(),
        reason: "test".to_string(),
        matched_signals: Vec::new(),
        status_label: "Typing".to_string(),
    };

    let metadata = chat_response_metadata(&response, Some(&dynamic_route), 30, &route_decision);
    assert_eq!(metadata["routingMode"], "dynamic");
    assert_eq!(metadata["eventKind"], "dynamic_routing");
    assert_eq!(metadata["executingModelId"], "gemma-4-12B-it-qat-q4_0-gguf");
    assert_eq!(metadata["targetModelId"], "gemma-4-12B-it-qat-q4_0-gguf");
    assert_eq!(metadata["routingClassifierSource"], "local_difficulty_v2");
    assert_eq!(metadata["routingCapability"], "general");
    assert_eq!(metadata["routingDemand"], "routine");
    assert_eq!(metadata["routingConfidence"], "confident");
    assert_eq!(
        metadata["routingClassificationReason"],
        "bounded_transformation"
    );
    assert!(metadata.get("promptComplexityScore").is_none());
    assert!(metadata.get("complexityThreshold").is_none());
    assert_eq!(metadata["gatewayExecutionLatencyMs"], 30);
    assert_eq!(metadata["promptTokens"], 17);
    assert_eq!(metadata["completionTokens"], 3);
    assert!(metadata.get("localInferenceUsage").is_none());
    assert!(serde_json::to_value(&response)
        .expect("response serializes")
        .get("local_usage")
        .is_none());
}

#[test]
fn response_integrity_flags_cutoff_fragments() {
    let mut response = InferenceResponse {
            provider_id: "prov-3".to_string(),
            provider: "Google Gemini".to_string(),
            model_id: "gemini-3.5-flash".to_string(),
            text: "Hello, Alex. It is a pleasure to connect. I am fully online, structured, and prepared to".to_string(),
            response_id: None,
            finish_reason: None,
            latency_ms: 0,
            local_usage: None,
        };
    assert_eq!(
        response_integrity_retry_reason(&response),
        Some("truncated_fragment")
    );

    response.text = "No stroke, Alex. I am fully restored, stable,".to_string();
    assert_eq!(
        response_integrity_retry_reason(&response),
        Some("truncated_fragment")
    );

    response.text = "Complete answer.".to_string();
    response.finish_reason = Some("MAX_TOKENS".to_string());
    assert_eq!(
        response_integrity_retry_reason(&response),
        Some("finish_reason_token_limit")
    );
}

#[test]
fn response_integrity_accepts_complete_short_answers() {
    for text in ["Yes.", "No.", "Complete answer.", "I can help with that."] {
        assert!(
            !looks_like_truncated_assistant_response(text),
            "complete short answer should not be treated as truncated: {text}"
        );
    }
}

#[test]
fn verified_history_rejects_fabricated_context_denial() {
    for denial in [
        "I can't access previous messages, so I cannot identify the quote.",
        "I can't access that quote.",
        "I don't have enough context to answer.",
        "I can't recall what that refers to.",
    ] {
        let response = InferenceResponse {
            provider_id: "local_model".to_string(),
            provider: "Local".to_string(),
            model_id: "gemma-4-12b".to_string(),
            text: denial.to_string(),
            response_id: None,
            finish_reason: Some("stop".to_string()),
            latency_ms: 0,
            local_usage: None,
        };

        assert_eq!(
            chat_response_retry_reason(
                &response,
                "What movie and who said that?",
                true,
                false,
                &[],
            ),
            Some("fabricated_history_unavailable"),
            "verified referential denial must be repaired: {denial}"
        );
        assert!(!fabricated_history_unavailable_claim(
            &response.text,
            "What movie and who said that?",
            false,
        ));
    }
}

#[test]
fn unrelated_capability_limits_are_not_misclassified_as_history_denials() {
    for capability_statement in [
        "I can't access the internet, but I can answer from our conversation.",
        "I can't access that website from this local session.",
        "I don't have access to your private calendar.",
    ] {
        assert!(!fabricated_history_unavailable_claim(
            capability_statement,
            "What movie and who said that?",
            true,
        ));
    }
}

#[test]
fn recent_context_repair_prompt_requires_direct_replacement_answer() {
    let prompt = response_integrity_repair_system_prompt(
        "base system",
        "fabricated_history_unavailable",
        None,
        &[],
    );

    assert!(prompt.contains("base system"));
    assert!(prompt.contains("Backend Recent-Context Repair"));
    assert!(prompt.contains("Verified Recent Conversation Reference"));
    assert!(prompt.contains("Return only the replacement answer"));
    assert!(!prompt.contains("made it look incomplete"));
}

#[test]
fn grounded_browser_action_narration_is_rejected_and_repaired_headlessly() {
    let response = InferenceResponse {
        provider_id: "gemini".to_string(),
        provider: "Google Gemini".to_string(),
        model_id: "gemini-test".to_string(),
        text: "To resolve this, I am launching the Sovereign Web Browser panel now.".to_string(),
        response_id: None,
        finish_reason: Some("stop".to_string()),
        latency_ms: 0,
        local_usage: None,
    };

    assert_eq!(
        chat_response_retry_reason(&response, "/research public facts", false, true, &[]),
        Some("grounded_browser_action_claim")
    );
    assert_eq!(
        chat_response_retry_reason(&response, "Open the browser", false, false, &[]),
        None,
        "the headless guard is scoped to factual-grounding turns"
    );

    let prompt = response_integrity_repair_system_prompt(
        "base system",
        "grounded_browser_action_claim",
        None,
        &[],
    );
    assert!(prompt.contains("Backend Headless-Grounding Repair"));
    assert!(prompt.contains("categorically headless"));
    assert!(prompt.contains("using only verified task-specific facts"));
    assert!(!grounded_browser_action_claim(
        "The approved sources did not include itinerary-specific prices."
    ));
}

#[test]
fn grounded_future_search_promises_cannot_become_terminal_answers() {
    for text in [
        "I am issuing the search now.",
        "I'll search the web and come back with sources.",
        "Let me research this online.",
        "Starting another public search.",
    ] {
        let response = InferenceResponse {
            provider_id: "gemini".to_string(),
            provider: "Google Gemini".to_string(),
            model_id: "gemini-test".to_string(),
            text: text.to_string(),
            response_id: None,
            finish_reason: Some("stop".to_string()),
            latency_ms: 0,
            local_usage: None,
        };
        assert_eq!(
            chat_response_retry_reason(&response, "Research this online", false, true, &[]),
            Some("grounded_search_promise"),
            "{text}"
        );
    }
    assert!(!prospective_search_promise(
        "The search returned two official sources, cited below."
    ));
}

#[test]
fn repair_runtime_settings_expand_output_budget() {
    let route = ResolvedProviderRoute {
        route_provider_id: "gemini".to_string(),
        catalog_provider_id: "gemini".to_string(),
        overrides: ProviderRouteOverrides::default(),
    };

    let medium = repair_runtime_settings_for_model_reasoning(&route, "gemini-3.5-flash", "medium");
    assert_eq!(medium.max_tokens, Some(REPAIR_MIN_OUTPUT_TOKENS));
    assert!(medium
        .temperature
        .is_some_and(|temperature| temperature <= 0.2));

    let ultra = repair_runtime_settings_for_model_reasoning(&route, "gemini-3.5-flash", "ultra");
    assert!(ultra
        .max_tokens
        .is_some_and(|tokens| tokens >= REPAIR_MIN_OUTPUT_TOKENS));
    assert!(ultra
        .max_tokens
        .is_some_and(|tokens| tokens <= REPAIR_MAX_OUTPUT_TOKENS));
    assert!(ultra
        .temperature
        .is_some_and(|temperature| temperature <= 0.2));
}

#[test]
fn persona_conflict_repair_runtime_settings_use_low_temperature() {
    let route = ResolvedProviderRoute {
        route_provider_id: "openai".to_string(),
        catalog_provider_id: "openai".to_string(),
        overrides: ProviderRouteOverrides::default(),
    };

    let settings =
        persona_conflict_repair_runtime_settings_for_model_reasoning(&route, "gpt-5", "high");

    assert!(settings
        .temperature
        .is_some_and(|temperature| temperature <= 0.1));
    assert!(settings
        .max_tokens
        .is_some_and(|tokens| tokens >= REPAIR_MIN_OUTPUT_TOKENS));
}

#[test]
fn salvage_incomplete_provider_response_preserves_non_empty_text() {
    let salvaged =
        salvage_incomplete_provider_response("The useful answer begins with", "truncated_fragment")
            .expect("non-empty text should be salvageable");

    assert!(salvaged.contains("The useful answer begins with ..."));
    assert!(salvaged.contains("provider stopped mid-response"));
    assert!(!looks_like_truncated_assistant_response(&salvaged));

    let code_salvage =
        salvage_incomplete_provider_response("```rust\nfn main() {", "truncated_fragment")
            .expect("non-empty code text should be salvageable");
    assert_eq!(code_salvage.matches("```").count() % 2, 0);
    assert!(code_salvage.contains("Note:"));

    assert!(salvage_incomplete_provider_response("   ", "truncated_fragment").is_none());
}

#[test]
fn truncated_assistant_history_is_filtered_from_model_context() {
    let filtered = filter_truncated_assistant_context(vec![
        test_message("user", "Hello OOMU"),
        test_message(
            "assistant",
            "Hello, Alex. I am fully online, structured, and prepared to",
        ),
        test_message("assistant", "I am ready now."),
    ]);

    let contents = filtered
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>();

    assert_eq!(contents, ["Hello OOMU", "I am ready now."]);
}

#[test]
fn pundamentals_missing_signal_builds_real_inference_repair_contract() {
    let context = crate::security::mods::ActiveModPromptContext {
        prompt: "Mod: Pundamentals\nRequired behavior:\nAdd one contextual pun.".to_string(),
        applied_mod_ids: vec!["ai.eldris.mods.pundamentals".to_string()],
        selection_mode: "agent_binding",
    };

    let repair_prompt = active_mod_compliance_repair_system_prompt("base system", &context);

    assert!(repair_prompt.contains("base system"));
    assert!(repair_prompt.contains("Generate a fresh, complete answer"));
    assert!(repair_prompt.contains("context-specific pun or wordplay"));
    assert!(!repair_prompt.contains("table manners"));
}

#[test]
fn pundamentals_signal_detection_accepts_real_model_wordplay() {
    let response = "The fix is in place, pun intended.";
    assert!(has_obvious_pundamentals_signal(response));
    assert!(!has_obvious_pundamentals_signal(
        "The fix is in place and the response is complete."
    ));
}

#[test]
fn local_chat_prompt_cleans_and_wraps_assistant_history() {
    let messages = vec![
        InferenceMessage {
            role: "assistant".to_string(),
            content: "<|channel>thought\n<channel|>I am OOMU.".to_string(),
            attachments: Vec::new(),
        },
        InferenceMessage {
            role: "user".to_string(),
            content: "Which model are you running?".to_string(),
            attachments: Vec::new(),
        },
    ];

    let settings = runtime_settings_for_reasoning(Some("medium"));
    let prompt = format_local_chat_prompt("session-test", "sys_inst", &messages, None, &settings);
    let parsed: serde_json::Value = serde_json::from_str(&prompt).unwrap();

    assert_eq!(parsed["sessionId"], "session-test");
    assert_eq!(parsed["systemPrompt"], "sys_inst");
    assert_eq!(parsed["messages"][0]["role"], "assistant");
    assert_eq!(
        parsed["messages"][0]["content"],
        "<|channel>thought\n<channel|>I am OOMU."
    );
    assert_eq!(parsed["messages"][1]["role"], "user");
    assert_eq!(
        parsed["messages"][1]["content"],
        "Which model are you running?"
    );
}

#[test]
fn local_chat_prompt_carries_approved_image_pixels_in_typed_media() {
    let encoded = base64::engine::general_purpose::STANDARD.encode(b"real-image-bytes");
    let messages = vec![InferenceMessage {
        role: "user".to_string(),
        content: "Describe this image.".to_string(),
        attachments: vec![ChatAttachment {
            name: "raven.png".to_string(),
            mime_type: "image/png".to_string(),
            byte_count: b"real-image-bytes".len(),
            data_base64: Some(encoded.clone()),
            text: Some("Supporting local visual context.".to_string()),
            approved_file_receipt: None,
        }],
    }];

    let settings = runtime_settings_for_reasoning(Some("medium"));
    let prompt = format_local_chat_prompt("session-image", "sys_inst", &messages, None, &settings);
    let parsed: serde_json::Value = serde_json::from_str(&prompt).unwrap();

    assert_eq!(parsed["messages"][0]["media"][0]["name"], "raven.png");
    assert_eq!(parsed["messages"][0]["media"][0]["mimeType"], "image/png");
    assert_eq!(parsed["messages"][0]["media"][0]["dataBase64"], encoded);
    assert!(parsed["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Supporting local visual context."));
}

#[test]
fn reasoning_on_uses_local_thinking_runtime_budget() {
    let settings = runtime_settings_for_reasoning(Some("on"));

    assert_eq!(settings.temperature, Some(0.2));
    assert_eq!(settings.max_tokens, Some(2_048));
}

#[test]
fn translate_reasoning_parameter_maps_provider_native_terms() {
    assert_eq!(
        translate_reasoning_parameter("google", "max"),
        ("xhigh".to_string(), Some(16_000))
    );
    assert_eq!(
        translate_reasoning_parameter("gemini", "xhigh"),
        ("xhigh".to_string(), Some(16_000))
    );
    assert_eq!(
        translate_reasoning_parameter("openai", "max"),
        ("max".to_string(), Some(16_000))
    );
    assert_eq!(
        translate_reasoning_parameter("anthropic", "medium"),
        ("medium".to_string(), Some(4_000))
    );
    assert_eq!(
        translate_reasoning_parameter("openrouter", "high"),
        ("high".to_string(), Some(8_000))
    );
    assert_eq!(
        translate_reasoning_parameter("qwen", "max"),
        ("max".to_string(), Some(16_000))
    );
    assert_eq!(
        translate_reasoning_parameter("local_model", "medium"),
        ("on".to_string(), Some(1))
    );
}

#[test]
fn runtime_settings_attach_translated_reasoning_budget() {
    let gemini_route = ResolvedProviderRoute {
        route_provider_id: "google".to_string(),
        catalog_provider_id: "google".to_string(),
        overrides: ProviderRouteOverrides::default(),
    };
    let gemini = runtime_settings_for_model_reasoning(&gemini_route, "gemini-3.5-flash", "max");
    assert_eq!(gemini.native_reasoning, Some("xhigh"));
    assert_eq!(gemini.reasoning_budget_tokens, Some(16_000));

    let claude_route = ResolvedProviderRoute {
        route_provider_id: "anthropic".to_string(),
        catalog_provider_id: "anthropic".to_string(),
        overrides: ProviderRouteOverrides::default(),
    };
    let claude = runtime_settings_for_model_reasoning(&claude_route, "claude-fable-5", "medium");
    assert_eq!(claude.native_reasoning, Some("medium"));
    assert_eq!(claude.reasoning_budget_tokens, Some(4_000));

    let local_route = ResolvedProviderRoute {
        route_provider_id: "local_model".to_string(),
        catalog_provider_id: "local_model".to_string(),
        overrides: ProviderRouteOverrides::default(),
    };
    let local = runtime_settings_for_model_reasoning(&local_route, "gemma-4-e2b", "medium");
    assert_eq!(local.native_reasoning, Some("on"));
    assert_eq!(local.reasoning_budget_tokens, Some(1));
}

#[test]
fn agent_output_token_limit_overrides_reasoning_runtime_budget() {
    let settings = runtime_settings_with_output_token_limit(
        runtime_settings_for_reasoning(Some("medium")),
        8_192,
    );
    assert_eq!(settings.temperature, Some(0.2));
    assert_eq!(settings.max_tokens, Some(8_192));

    let low = runtime_settings_with_output_token_limit(
        runtime_settings_for_reasoning(Some("ultra")),
        512,
    );
    assert_eq!(low.max_tokens, Some(MIN_AGENT_MAX_OUTPUT_TOKENS as u32));

    let snapped = runtime_settings_with_output_token_limit(
        runtime_settings_for_reasoning(Some("medium")),
        7_600,
    );
    assert_eq!(snapped.max_tokens, Some(7_168));
}

#[test]
fn local_chat_prompt_preserves_active_mod_runtime_contract() {
    let messages = vec![InferenceMessage {
        role: "user".to_string(),
        content: "Hello OOMU. How are you?".to_string(),
        attachments: Vec::new(),
    }];

    let settings = runtime_settings_for_reasoning(Some("medium"));
    let active_mod_context = "Active OOMU Mod Runtime Contract\nStatus: mandatory for this turn.\n\nActive OOMU Mod Prompt Hooks\nMod: Pundamentals\nRequired behavior:\nAdd one contextual pun.";
    let system_prompt = format!(
        "{active_mod_context}\n\n{}",
        active_mod_enforcement_reminder(active_mod_context)
    );
    let prompt =
        format_local_chat_prompt("session-test", &system_prompt, &messages, None, &settings);
    let parsed: serde_json::Value = serde_json::from_str(&prompt).unwrap();
    let serialized_system_prompt = parsed["systemPrompt"].as_str().unwrap();

    assert!(serialized_system_prompt.contains("Active OOMU Mod Runtime Contract"));
    assert!(serialized_system_prompt.contains("Status: mandatory for this turn."));
    assert!(serialized_system_prompt.contains("Mod: Pundamentals"));
    assert!(serialized_system_prompt.contains("Add one contextual pun."));
    assert!(serialized_system_prompt.contains("Active OOMU Mod Enforcement Reminder"));
    assert_eq!(parsed["messages"][0]["content"], "Hello OOMU. How are you?");
}
