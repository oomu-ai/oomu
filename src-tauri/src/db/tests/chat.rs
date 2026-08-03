use super::*;

#[test]
fn semantic_compaction_preserves_six_recent_turn_pairs_and_writes_older_anchor() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_semantic_compaction_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    for turn in 0..8 {
        engine
            .insert_chat_message(
                "session-a",
                "agent-a",
                "user",
                &format!("User turn {turn}: keep this conversation coherent."),
            )
            .unwrap();
        let assistant = if turn == 6 {
            format!(
                "{} RECENT-REFERENCE-CANARY: Wooderson said the quoted line.",
                "bounded context ".repeat(48)
            )
        } else {
            format!("Assistant turn {turn}: acknowledged.")
        };
        engine
            .insert_chat_message("session-a", "agent-a", "assistant", &assistant)
            .unwrap();
    }

    let response = engine.compact_session_messages("session-a").unwrap();
    assert_eq!(response.compacted_messages, 4);
    assert!(response.anchor_message_id.is_some());

    let active = engine.select_chat_messages("session-a").unwrap();
    assert_eq!(active.len(), 13);
    assert_eq!(active[0].role, "system");
    assert_eq!(active[0].compaction_type.as_deref(), Some("summary_anchor"));
    assert!(active[0]
        .content
        .contains("no goals, conclusions, or pending tasks were inferred"));
    assert!(active[0].content.contains("role=user sha256="));
    assert!(active[0].content.contains("role=assistant sha256="));
    assert_eq!(
        active
            .iter()
            .filter(|message| message.role.eq_ignore_ascii_case("user"))
            .count(),
        RECENT_RAW_CHAT_TURNS_TO_PRESERVE
    );
    assert!(active
        .iter()
        .any(|message| message.content.contains("RECENT-REFERENCE-CANARY")));
    assert!(active[0].created_at_ms < active[1].created_at_ms);

    let history = engine.get_chat_history("session-a", 20).unwrap();
    assert_eq!(history.len(), 13);
    assert_eq!(history[0].content, active[0].content);
    assert!(!history[0].content.contains("Active Goals:"));
    assert!(history
        .iter()
        .any(|message| message.content.contains("RECENT-REFERENCE-CANARY")));

    let connection = engine.open_connection().unwrap();
    let compacted_count: i64 = connection
        .query_row(
            "
                SELECT COUNT(*)
                FROM chat_messages
                WHERE session_id = 'session-a' AND is_compacted = 1
                ",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(compacted_count, 4);

    drop(connection);
    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn session_context_policy_preserves_explicit_budget_and_records_manual_compaction() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_session_context_policy_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    engine
        .upsert_session_config(
            "session-policy",
            "high",
            32_768,
            Some("gemini"),
            Some("gemini"),
            Some("gemini-3.6-flash"),
        )
        .unwrap();
    let saved = engine
        .save_session_context_policy(&SaveSessionContextPolicyRequest {
            session_id: "session-policy".to_string(),
            auto_compaction_threshold_percent: 60,
            auto_compaction_enabled: true,
        })
        .unwrap();
    assert_eq!(saved.auto_compaction_threshold_percent, 60);
    assert!(saved.auto_compaction_enabled);
    assert_eq!(
        engine
            .select_session_config("session-policy")
            .unwrap()
            .unwrap()
            .context_budget,
        32_768
    );
    let ui_checkpoint_id = engine
        .insert_chat_message_with_metadata(
            "session-policy",
            "agent-a",
            "system",
            "Approval is waiting for the user.",
            None,
            None,
            Some(&json!({"uiOnlyCheckpoint": true, "approvalToken": "approval-1"})),
        )
        .unwrap();
    let unresolved_message_id = engine
        .insert_chat_message_with_metadata(
            "session-policy",
            "agent-a",
            "user",
            "Continue the approved write after OOMU restarts.",
            None,
            None,
            Some(&json!({"turnState": "accepted", "turnId": "turn-held-write"})),
        )
        .unwrap();

    for turn in 0..8 {
        engine
            .insert_chat_message(
                "session-policy",
                "agent-a",
                "user",
                &format!("User turn {turn}: {}", "evidence ".repeat(32)),
            )
            .unwrap();
        engine
            .insert_chat_message(
                "session-policy",
                "agent-a",
                "assistant",
                &format!("Assistant turn {turn}: verified."),
            )
            .unwrap();
    }
    let result = engine
        .compact_chat_session(&CompactChatSessionRequest {
            session_id: "session-policy".to_string(),
            target_percent: Some(60),
        })
        .unwrap();
    assert_eq!(result.compacted_message_count, 4);
    assert_eq!(result.preserved_message_count, 14);
    assert_eq!(result.target_tokens, 19_660);
    assert!(result.after_tokens < result.before_tokens);

    let status = engine.session_context_status("session-policy").unwrap();
    assert_eq!(status.working_budget_tokens, 32_768);
    assert!(status.provider_max_tokens >= status.working_budget_tokens);
    assert_eq!(status.auto_compaction_threshold_percent, 60);
    assert_eq!(status.last_compaction.unwrap().session_id, "session-policy");
    let connection = engine.open_connection().unwrap();
    let checkpoint_still_active: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM chat_messages WHERE id = ?1 AND is_compacted = 0",
            params![ui_checkpoint_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(checkpoint_still_active, 1);
    let unresolved_still_active: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM chat_messages WHERE id = ?1 AND is_compacted = 0",
            params![unresolved_message_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unresolved_still_active, 1);
    drop(connection);

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn chat_queries_reject_foreign_workspace_rows() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_chat_workspace_firewall_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let foreign_workspace_id = crate::security::firewall::workspace_id_for_root("/tmp/eldris");
    let connection = engine.open_connection().unwrap();
    connection
            .execute(
                "
                INSERT INTO chat_sessions (
                    id, workspace_id, agent_id, title, provider_id, model_id, created_at_ms, updated_at_ms
                )
                VALUES ('foreign-session', ?1, 'agent-a', 'Foreign', 'local_model', 'gemma', 1, 1)
                ",
                params![foreign_workspace_id],
            )
            .unwrap();
    connection
        .execute(
            "
                INSERT INTO chat_messages (
                    workspace_id, session_id, agent_id, role, content, timestamp_ms
                )
                VALUES (?1, 'foreign-session', 'agent-a', 'user', 'Eldris database credentials', 1)
                ",
            params![foreign_workspace_id],
        )
        .unwrap();
    drop(connection);

    assert!(engine.select_chat_sessions().unwrap().is_empty());
    assert!(engine.select_chat_session_by_id("foreign-session").is_err());
    assert!(engine.select_chat_messages("foreign-session").is_err());
    let blocks = engine
        .search_relevant_chat_memory_blocks(
            None,
            "agent-a",
            "Eldris database credentials",
            None,
            10,
        )
        .unwrap();
    assert!(blocks.is_empty());

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn active_trust_session_matches_only_its_session_and_expiration_window() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_trust_session_{}", unix_time_ms()));
    let trusted_dir = temp_dir.join("strategy");
    std::fs::create_dir_all(&trusted_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let now_ms = unix_time_ms();

    let session_grant_id = engine
        .activate_sovereign_trust_session(
            "chat-123",
            trusted_dir.to_str().unwrap(),
            &[SovereignTrustToolCategory::ExternalWrites],
            Some(now_ms + SOVEREIGN_TRUST_SESSION_DURATION_MS),
            None,
            None,
        )
        .unwrap();
    assert!(!session_grant_id.is_empty());

    let target = trusted_dir.join("plan.md");
    assert!(engine
        .select_matching_sovereign_trust_grant(
            Some("chat-123"),
            &target,
            SovereignTrustToolCategory::ExternalWrites,
            now_ms,
        )
        .unwrap()
        .is_some());
    assert!(engine
        .select_matching_sovereign_trust_grant(
            Some("other-chat"),
            &target,
            SovereignTrustToolCategory::ExternalWrites,
            now_ms,
        )
        .unwrap()
        .is_none());
    assert!(engine
        .select_matching_sovereign_trust_grant(
            Some("chat-123"),
            &target,
            SovereignTrustToolCategory::ExternalWrites,
            now_ms + SOVEREIGN_TRUST_SESSION_DURATION_MS + 1,
        )
        .unwrap()
        .is_none());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn database_initialization_returns_error_for_unusable_parent_path() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_bad_db_parent_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let file_parent = temp_dir.join("not-a-directory");
    std::fs::write(&file_parent, b"occupied").unwrap();

    let error = match PersistenceEngine::initialize_at(file_parent.join("state.sqlite")) {
        Ok(_) => panic!("database initialization should fail without panicking"),
        Err(error) => error,
    };

    assert!(
        error.contains("Not a directory")
            || error.contains("not a directory")
            || error.contains("File exists"),
        "unexpected initialization error: {error}"
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn user_renamed_chat_session_resists_auto_title_touch() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_chat_title_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "local_model".to_string(),
            model_id: crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            title: None,
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    assert_eq!(session.title, "New Session");
    assert_eq!(session.title_source, "auto");

    let renamed = engine.rename_chat_session(&session.id, "Test 5.0").unwrap();
    assert_eq!(renamed.title, "Test 5.0");
    assert_eq!(renamed.title_source, "user");

    engine
        .touch_chat_session(
            &session.id,
            Some("OOMU perform a system audit"),
            "local_model",
            "gemma-test",
        )
        .unwrap();
    let selected = engine.select_chat_session_by_id(&session.id).unwrap();
    assert_eq!(selected.title, "Test 5.0");
    assert_eq!(selected.title_source, "user");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn queued_chat_turn_context_roundtrips_and_rejects_missing_or_cross_route_context() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_chat_turn_queue_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-216".to_string(),
            provider_id: "provider-216".to_string(),
            model_id: "model-216".to_string(),
            title: Some("Turn context test".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let root = ChatTurnPersistenceContext {
        turn_id: "turn-root-216".to_string(),
        generation_token: "generation-root-216".to_string(),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        parent_turn_id: None,
        root_turn_id: "turn-root-216".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.begin_chat_turn(&root).unwrap();

    let request = QueueMessageRequest {
        turn_id: Some("turn-queued-216".to_string()),
        generation_token: Some("generation-queued-216".to_string()),
        parent_turn_id: Some(root.turn_id.clone()),
        root_turn_id: Some(root.root_turn_id.clone()),
        turn_kind: Some("queued".to_string()),
        agent_id: root.agent_id.clone(),
        message: "continue with the queued request".to_string(),
        attachments: Vec::new(),
        session_id: Some(root.session_id.clone()),
        provider_id: Some(root.provider_id.clone()),
        model_id: Some(root.model_id.clone()),
        reasoning: Some("balanced".to_string()),
        context: Some("8192".to_string()),
        context_budget: None,
        steering: None,
        automated_web_grounding_enabled: Some(true),
        dynamic_routing_override: Some(false),
    };
    let queued = engine.insert_queued_message(request.clone()).unwrap();
    assert_eq!(queued.turn_id.as_deref(), Some("turn-queued-216"));
    assert_eq!(
        queued.generation_token.as_deref(),
        Some("generation-queued-216")
    );
    assert_eq!(queued.parent_turn_id.as_deref(), Some("turn-root-216"));
    assert_eq!(queued.root_turn_id.as_deref(), Some("turn-root-216"));
    assert_eq!(queued.turn_kind.as_deref(), Some("queued"));
    assert_eq!(queued.automated_web_grounding_enabled, Some(true));
    assert_eq!(queued.dynamic_routing_override, Some(false));
    let selected = engine.select_queued_messages(&session.id).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].turn_id, queued.turn_id);
    assert_eq!(selected[0].generation_token, queued.generation_token);

    let mut oversized_attachments = request.clone();
    oversized_attachments.attachments = vec![
        crate::inference::ChatAttachment {
            name: "bounded.txt".to_string(),
            mime_type: "text/plain".to_string(),
            byte_count: 0,
            data_base64: None,
            text: None,
            approved_file_receipt: None,
        };
        crate::inference::MAX_CHAT_ATTACHMENTS + 1
    ];
    let queue_error = engine
        .insert_queued_message(oversized_attachments)
        .expect_err("oversized attachment batch must be rejected before serialization");
    assert!(matches!(
        queue_error,
        rusqlite::Error::InvalidParameterName(ref code)
            if code == "attachment_count_limit_exceeded"
    ));
    let queued_after_rejection = engine.select_queued_messages(&session.id).unwrap();
    assert_eq!(queued_after_rejection.len(), 1);

    let mut dynamic_child = request.clone();
    dynamic_child.turn_id = Some("turn-dynamic-child-216".to_string());
    dynamic_child.generation_token = Some("generation-dynamic-child-216".to_string());
    dynamic_child.provider_id = Some("dynamic".to_string());
    dynamic_child.model_id = Some("dynamic".to_string());
    let resolved_dynamic_child = engine.insert_queued_message(dynamic_child).unwrap();
    assert_eq!(
        resolved_dynamic_child.provider_id.as_deref(),
        Some("provider-216")
    );
    assert_eq!(
        resolved_dynamic_child.model_id.as_deref(),
        Some("model-216")
    );

    let mut unresolved_dynamic_root = request.clone();
    unresolved_dynamic_root.turn_id = Some("turn-dynamic-root-216".to_string());
    unresolved_dynamic_root.generation_token = Some("generation-dynamic-root-216".to_string());
    unresolved_dynamic_root.parent_turn_id = None;
    unresolved_dynamic_root.root_turn_id = Some("turn-dynamic-root-216".to_string());
    unresolved_dynamic_root.turn_kind = Some("root".to_string());
    unresolved_dynamic_root.provider_id = Some("dynamic".to_string());
    unresolved_dynamic_root.model_id = Some("dynamic".to_string());
    assert!(engine
        .insert_queued_message(unresolved_dynamic_root)
        .is_err());

    let mut missing_generation = request.clone();
    missing_generation.generation_token = None;
    assert!(engine.insert_queued_message(missing_generation).is_err());

    let other_session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-216".to_string(),
            provider_id: "provider-216".to_string(),
            model_id: "model-216".to_string(),
            title: Some("Other queue".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let other_root = ChatTurnPersistenceContext {
        turn_id: "turn-other-root-216".to_string(),
        generation_token: "generation-other-root-216".to_string(),
        session_id: other_session.id.clone(),
        agent_id: other_session.agent_id.clone(),
        provider_id: other_session.provider_id.clone(),
        model_id: other_session.model_id.clone(),
        parent_turn_id: None,
        root_turn_id: "turn-other-root-216".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.begin_chat_turn(&other_root).unwrap();
    let mut other_request = request.clone();
    other_request.turn_id = Some("turn-other-queued-216".to_string());
    other_request.generation_token = Some("generation-other-queued-216".to_string());
    other_request.parent_turn_id = Some(other_root.turn_id.clone());
    other_request.root_turn_id = Some(other_root.root_turn_id.clone());
    other_request.session_id = Some(other_session.id.clone());
    engine.insert_queued_message(other_request).unwrap();
    let claimed = engine.claim_queued_messages(&session.id, 100).unwrap();
    assert!(!claimed.is_empty());
    assert!(claimed
        .iter()
        .all(|record| record.session_id.as_deref() == Some(session.id.as_str())));
    let other_pending = engine.select_queued_messages(&other_session.id).unwrap();
    assert_eq!(other_pending.len(), 1);
    assert_eq!(
        other_pending[0].session_id.as_deref(),
        Some(other_session.id.as_str())
    );

    let mut changed_route = request;
    changed_route.turn_id = Some("turn-cross-route-216".to_string());
    changed_route.generation_token = Some("generation-cross-route-216".to_string());
    changed_route.model_id = Some("other-model".to_string());
    assert!(engine.insert_queued_message(changed_route).is_err());

    let mut stale_generation = root.clone();
    stale_generation.generation_token = "stale-generation".to_string();
    assert!(engine
        .validate_chat_turn_generation(&stale_generation)
        .is_err());
    assert!(engine
        .finish_chat_turn(&stale_generation, "completed")
        .is_err());

    assert!(engine.delete_chat_session_by_id(&session.id).unwrap());
    let connection = engine.open_connection().unwrap();
    let root_status: String = connection
        .query_row(
            "SELECT status FROM chat_turns WHERE turn_id = ?1",
            params![root.turn_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(root_status, "cancelled");
    let queued_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM message_queue WHERE session_id = ?1",
            params![session.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(queued_count, 0);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn agent_execution_gets_private_project_without_changing_chat_routing_scope() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_agent_local_files_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-local-files".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Create a PDF".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let turn = ChatTurnPersistenceContext {
        turn_id: "turn-local-files".to_string(),
        generation_token: "generation-local-files".to_string(),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        parent_turn_id: None,
        root_turn_id: "turn-local-files".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.begin_chat_turn(&turn).unwrap();
    engine
        .begin_agent_execution("execution-local-files", "plan-local-files", &turn, "{}")
        .unwrap();

    let connection = engine.open_connection().unwrap();
    let (session_project, execution_project): (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT c.project_id,a.project_id FROM chat_sessions c JOIN agent_executions a ON a.session_id=c.id WHERE a.execution_id='execution-local-files'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
    assert!(session_project.is_none());
    assert_eq!(
        execution_project.as_deref(),
        Some(crate::projects::repository::INTERNAL_LOCAL_FILES_PROJECT_ID)
    );
    drop(connection);
    assert!(engine
        .project_inference_context_for_session(&session.id)
        .unwrap()
        .is_none());
    assert!(
        crate::projects::evaluate_project_provider_for_session(
            &engine,
            &session.id,
            "google_gemini",
            "google_gemini",
        )
        .unwrap()
        .allowed
    );

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn chat_session_web_grounding_override_persists() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_chat_grounding_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Privacy test".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    assert_eq!(session.web_grounding_override, None);
    assert_eq!(session.dynamic_routing_override, None);

    let updated = engine
        .update_chat_session_web_grounding_override(&session.id, Some(true))
        .unwrap();
    assert_eq!(updated.web_grounding_override, Some(true));
    assert_eq!(updated.dynamic_routing_override, None);

    let sessions = engine.select_chat_sessions().unwrap();
    assert_eq!(sessions[0].web_grounding_override, Some(true));
    let selected = engine.select_chat_session_by_id(&session.id).unwrap();
    assert_eq!(selected.web_grounding_override, Some(true));
    assert_eq!(selected.dynamic_routing_override, None);

    let reset = engine
        .update_chat_session_web_grounding_override(&session.id, None)
        .unwrap();
    assert_eq!(reset.web_grounding_override, None);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn chat_session_dynamic_routing_override_persists() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_chat_routing_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Routing test".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    engine
        .upsert_session_config(
            &session.id,
            "medium",
            8_192,
            Some("local_model"),
            Some("local_model"),
            Some(crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID),
        )
        .unwrap();
    assert_eq!(session.dynamic_routing_override, None);

    let enabled = engine
        .update_chat_session_dynamic_routing_override(
            &session.id,
            Some(true),
            Some(test_verified_auto_route_baseline(
                crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
            )),
            Some(&installed_model_root()),
        )
        .unwrap();
    assert_eq!(enabled.session.dynamic_routing_override, Some(true));

    let disabled = engine
        .update_chat_session_dynamic_routing_override(
            &session.id,
            Some(false),
            Some(test_verified_auto_route_baseline(
                crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
            )),
            Some(&installed_model_root()),
        )
        .unwrap();
    assert_eq!(disabled.session.dynamic_routing_override, Some(false));

    let sessions = engine.select_chat_sessions().unwrap();
    assert_eq!(sessions[0].dynamic_routing_override, Some(false));
    let selected = engine.select_chat_session_by_id(&session.id).unwrap();
    assert_eq!(selected.dynamic_routing_override, Some(false));

    let reset = engine
        .update_chat_session_dynamic_routing_override(
            &session.id,
            None,
            Some(test_verified_auto_route_baseline(
                crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
            )),
            Some(&installed_model_root()),
        )
        .unwrap();
    assert_eq!(reset.session.dynamic_routing_override, None);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn create_chat_session_accepts_dynamic_routing_default() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_chat_routing_default_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-dev".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: Some("Developer session".to_string()),
            dynamic_routing_override: Some(true),
            workspace_id: None,
        })
        .unwrap();
    assert_eq!(session.dynamic_routing_override, Some(true));

    let selected = engine.select_chat_session_by_id(&session.id).unwrap();
    assert_eq!(selected.dynamic_routing_override, Some(true));
    let sessions = engine.select_chat_sessions().unwrap();
    assert_eq!(sessions[0].dynamic_routing_override, Some(true));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn dynamic_chat_session_keeps_binding_with_message_and_audit_metadata() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_dynamic_route_metadata_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: Some("Auto route".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();

    let metadata = json!({
        "routingMode": "dynamic",
        "promptComplexityScore": 0.12,
        "targetModelId": "gemma-4-E2B-it-qat-q4_0-gguf",
        "gatewayExecutionLatencyMs": 42,
    });
    engine
        .insert_chat_message_with_metadata(
            &session.id,
            "agent-test",
            "assistant",
            "Paris.",
            Some("local_model"),
            Some("gemma-4-E2B-it-qat-q4_0-gguf"),
            Some(&metadata),
        )
        .unwrap();
    engine
        .touch_chat_session(
            &session.id,
            Some("What is the capital of France?"),
            "dynamic",
            "dynamic",
        )
        .unwrap();
    let selected = engine.select_chat_session_by_id(&session.id).unwrap();
    assert_eq!(selected.provider_id, "dynamic");
    assert_eq!(selected.model_id, "dynamic");

    let messages = engine.select_chat_messages(&session.id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].provider_id.as_deref(), Some("local_model"));
    assert_eq!(
        messages[0].model_id.as_deref(),
        Some("gemma-4-E2B-it-qat-q4_0-gguf")
    );
    assert!(messages[0]
        .metadata_json
        .as_deref()
        .is_some_and(|value| value.contains("promptComplexityScore")));

    engine
        .insert_dynamic_routing_audit("What is the capital of France?", "Paris.", &metadata)
        .unwrap();
    let ops = engine.open_ops_connection().unwrap();
    let count: i64 = ops
        .query_row(
            "SELECT COUNT(*) FROM local_inference_audit WHERE event_kind = 'dynamic_routing'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn session_configs_persist_reasoning_context_and_route_fields() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_session_configs_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("session.sqlite")).unwrap();

    engine
        .upsert_session_config(
            "session-82",
            "high",
            8192,
            Some("provider-1"),
            Some("provider-1"),
            Some("model-1"),
        )
        .unwrap();

    let config = engine
        .select_session_config("session-82")
        .unwrap()
        .expect("session config should persist");
    assert_eq!(config.session_id, "session-82");
    assert_eq!(config.reasoning_depth, "high");
    assert_eq!(config.context_budget, 8192);
    assert_eq!(
        config.local_provider_config_id.as_deref(),
        Some("provider-1")
    );
    assert_eq!(config.local_provider_type.as_deref(), Some("provider-1"));
    assert_eq!(config.local_route_generation, 1);
    assert_eq!(config.model_id.as_deref(), Some("model-1"));

    engine
        .upsert_session_config("session-82", "medium", 2048, None, None, None)
        .unwrap();
    let updated = engine
        .select_session_config("session-82")
        .unwrap()
        .expect("session config should update");
    assert_eq!(updated.reasoning_depth, "medium");
    assert_eq!(updated.context_budget, 2048);
    assert_eq!(
        updated.local_provider_config_id.as_deref(),
        Some("provider-1")
    );
    assert_eq!(updated.local_provider_type.as_deref(), Some("provider-1"));
    assert_eq!(updated.local_route_generation, 1);
    assert_eq!(updated.model_id.as_deref(), Some("model-1"));

    drop(engine);
    let reopened = PersistenceEngine::initialize_at(temp_dir.join("session.sqlite")).unwrap();
    let restarted = reopened
        .select_session_config("session-82")
        .unwrap()
        .unwrap();
    assert_eq!(
        restarted.local_provider_config_id.as_deref(),
        Some("provider-1")
    );
    assert_eq!(restarted.local_provider_type.as_deref(), Some("provider-1"));
    assert_eq!(restarted.local_route_generation, 1);
    drop(reopened);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn session_config_reasoning_accepts_local_on_mode() {
    assert_eq!(
        clean_session_reasoning_depth(Some("local_model"), "on".to_string()).unwrap(),
        "on"
    );
    assert_eq!(
        clean_session_reasoning_depth(Some("google"), "max".to_string()).unwrap(),
        "max"
    );
    assert_eq!(
        clean_session_reasoning_depth(Some("google"), "ultra".to_string()).unwrap(),
        "max"
    );
}

#[test]
fn session_config_reasoning_uses_provider_specific_defaults() {
    assert_eq!(get_default_reasoning_depth_for_provider("google"), "medium");
    assert_eq!(get_default_reasoning_depth_for_provider("gemini"), "medium");
    assert_eq!(get_default_reasoning_depth_for_provider("openai"), "high");
    assert_eq!(
        get_default_reasoning_depth_for_provider("anthropic"),
        "high"
    );
    assert_eq!(
        get_default_reasoning_depth_for_provider("local_model"),
        "low"
    );
    assert_eq!(
        clean_session_reasoning_depth(Some("google"), "on".to_string()).unwrap(),
        "medium"
    );
    assert_eq!(
        clean_session_reasoning_depth(Some("gemini"), "unexpected".to_string()).unwrap(),
        "medium"
    );
    assert_eq!(
        clean_session_reasoning_depth(Some("openai"), "".to_string()).unwrap(),
        "high"
    );
    assert_eq!(
        clean_session_reasoning_depth(Some("local_model"), "".to_string()).unwrap(),
        "low"
    );
}
