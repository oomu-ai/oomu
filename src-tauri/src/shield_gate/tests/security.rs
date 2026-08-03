use super::*;

#[test]
fn volatile_persistence_blocks_direct_command_before_audit_side_effect() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-volatile-shield-{}", unix_time_ms_u64()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let persistence =
        PersistenceEngine::initialize_volatile_at(temp_dir.join("state.sqlite")).unwrap();

    let error = require_durable_direct_command(&persistence).unwrap_err();
    assert_eq!(error.code, "volatile_persistence_command_blocked");
    let action_count: i64 = persistence
        .open_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM actions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(action_count, 0);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn direct_command_turn_guard_rejects_missing_and_partial_context() {
    let temp_dir = std::env::temp_dir().join(format!("oomu-direct-context-{}", unix_time_ms_u64()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let mut request = ExecuteCommandRequest {
        action: RequestedAction {
            kind: "shell_command".to_string(),
            principal: None,
            path: None,
            content: Some("printf canary".to_string()),
        },
        logical_certificate: None,
        session_id: None,
        turn_id: None,
        generation_token: None,
        agent_id: None,
        provider_id: None,
        model_id: None,
        parent_turn_id: None,
        root_turn_id: None,
        turn_kind: None,
        project_id: None,
        task_run_id: None,
    };
    assert_eq!(
        DirectCommandTurnGuard::begin(&persistence, &request)
            .err()
            .unwrap()
            .code,
        "chat_turn_context_invalid"
    );
    request.session_id = Some("session-1".to_string());
    request.turn_id = Some("turn-1".to_string());
    assert_eq!(
        DirectCommandTurnGuard::begin(&persistence, &request)
            .err()
            .unwrap()
            .code,
        "chat_turn_context_invalid"
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn direct_command_turn_guard_closes_abandoned_requests_and_rejects_stale_context() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-direct-turn-guard-{}", unix_time_ms_u64()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = persistence
        .ensure_chat_session(crate::db::CreateChatSessionRequest {
            agent_id: "agent-guard".to_string(),
            provider_id: "provider-guard".to_string(),
            model_id: "model-guard".to_string(),
            title: Some("Guard test".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let mut request = ExecuteCommandRequest {
        action: RequestedAction {
            kind: "file_write".to_string(),
            principal: None,
            path: Some("/tmp/guard-test.txt".to_string()),
            content: Some("guarded".to_string()),
        },
        logical_certificate: None,
        session_id: Some(session.id.clone()),
        turn_id: Some("turn-guard".to_string()),
        generation_token: Some("generation-guard".to_string()),
        agent_id: Some(session.agent_id.clone()),
        provider_id: Some(session.provider_id.clone()),
        model_id: Some(session.model_id.clone()),
        parent_turn_id: None,
        root_turn_id: Some("turn-guard".to_string()),
        turn_kind: Some("root".to_string()),
        project_id: None,
        task_run_id: None,
    };
    let mut guard = DirectCommandTurnGuard::begin(&persistence, &request)
        .unwrap()
        .expect("chat-scoped direct action creates a guard");
    guard.validate_current().unwrap();
    guard
        .finalize_output(&ExecuteCommandResponse {
            operation: "file_read".to_string(),
            status: CommandStatus::Failed,
            message: "The approved file could not be viewed safely.".to_string(),
            metrics: None,
            claims: Vec::new(),
            verified: false,
            model_used: None,
        })
        .unwrap();
    drop(guard);
    let abandoned_status: String = persistence
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT status FROM chat_turns WHERE turn_id = ?1",
            ["turn-guard"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(abandoned_status, "failed");

    request.turn_id = Some("turn-stale".to_string());
    request.generation_token = Some("generation-stale".to_string());
    request.root_turn_id = Some("turn-stale".to_string());
    let guard = DirectCommandTurnGuard::begin(&persistence, &request)
        .unwrap()
        .expect("a subsequent direct action creates a fresh guard");
    guard.validate_current().unwrap();
    persistence.delete_chat_session_by_id(&session.id).unwrap();
    assert!(guard.validate_current().is_err());

    drop(guard);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn direct_command_turn_guard_resolves_dynamic_children_to_the_claimed_parent_route() {
    let temp_dir = std::env::temp_dir().join(format!("oomu-direct-dynamic-{}", unix_time_ms_u64()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = persistence
        .ensure_chat_session(crate::db::CreateChatSessionRequest {
            agent_id: "agent-dynamic".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: Some("Dynamic guard test".to_string()),
            dynamic_routing_override: Some(true),
            workspace_id: None,
        })
        .unwrap();
    let parent = ChatTurnPersistenceContext {
        turn_id: "turn-dynamic-parent".to_string(),
        generation_token: "generation-dynamic-parent".to_string(),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: "dynamic".to_string(),
        model_id: "dynamic".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-dynamic-parent".to_string(),
        turn_kind: "root".to_string(),
    };
    persistence.begin_chat_turn(&parent).unwrap();
    let mut claimed_parent = parent.clone();
    claimed_parent.provider_id = "provider-cloud".to_string();
    claimed_parent.model_id = "model-cloud".to_string();
    persistence
        .begin_or_claim_chat_turn_response(&claimed_parent)
        .unwrap();

    for kind in ["queued", "steer"] {
        let request = ExecuteCommandRequest {
            action: RequestedAction {
                kind: "file_read".to_string(),
                principal: None,
                path: Some("/tmp/dynamic-guard.txt".to_string()),
                content: None,
            },
            logical_certificate: None,
            session_id: Some(session.id.clone()),
            turn_id: Some(format!("turn-dynamic-{kind}")),
            generation_token: Some(format!("generation-dynamic-{kind}")),
            agent_id: Some(session.agent_id.clone()),
            provider_id: Some("dynamic".to_string()),
            model_id: Some("dynamic".to_string()),
            parent_turn_id: Some(parent.turn_id.clone()),
            root_turn_id: Some(parent.turn_id.clone()),
            turn_kind: Some(kind.to_string()),
            project_id: None,
            task_run_id: None,
        };
        let guard = DirectCommandTurnGuard::begin(&persistence, &request)
            .unwrap()
            .expect("dynamic child direct action creates a guard");
        guard.validate_current().unwrap();
        let child = persistence
            .select_chat_turn_context(request.turn_id.as_deref().unwrap())
            .unwrap()
            .expect("child turn is durably prebound");
        assert_eq!(child.provider_id, claimed_parent.provider_id);
        assert_eq!(child.model_id, claimed_parent.model_id);
        drop(guard);
    }

    let _ = persistence.finish_chat_turn(&claimed_parent, "completed");
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn strict_logical_certificate_bounds_accept_explicit_reasoning_shape() {
    let certificate = LogicalCertificate::unsigned(
        vec!["File read path and objective are declared.".to_string()],
        vec![
            "Validate the requested path is inside the app data quarantine.".to_string(),
            "Read the file through the bounded filesystem tool.".to_string(),
        ],
        "The file read remains inside ShieldGate constraints.".to_string(),
    );

    certificate
        .validate()
        .expect("strict MLC bounds should accept explicit reasoning");
}

#[test]
fn strict_action_plan_certificate_accepts_single_signed_step() {
    let certificate = LogicalCertificate::unsigned(
        vec![
            "objective=Search current public web context".to_string(),
            "plan_id=plan-single-step".to_string(),
        ],
        vec![
            "1. step=Search current public web context tool=sovereign_duckduckgo_search risk=Low"
                .to_string(),
        ],
        "Return verified source-grounded search context.".to_string(),
    );

    certificate
        .validate_for_action_kind("action_plan")
        .expect("single-step ActionPlans remain valid in strict mode");
}

#[test]
fn registered_system_tools_are_risk_stratified() {
    for (kind, expected_tier, expected_risk) in [
        (
            "file_write",
            ShieldToolApprovalTier::VisualConsent,
            "Medium Risk",
        ),
        (
            "codebase_patch",
            ShieldToolApprovalTier::VisualConsent,
            "Medium Risk",
        ),
        (
            "document_index",
            ShieldToolApprovalTier::VisualConsent,
            "Medium Risk",
        ),
        (
            "delete_file",
            ShieldToolApprovalTier::ExplicitConfirmation,
            "High Risk",
        ),
        (
            "trash",
            ShieldToolApprovalTier::ExplicitConfirmation,
            "High Risk",
        ),
        (
            "shell_command",
            ShieldToolApprovalTier::ExplicitConfirmation,
            "High Risk",
        ),
        (
            "network_request",
            ShieldToolApprovalTier::ExplicitConfirmation,
            "High Risk",
        ),
        (
            "mcp_connect_server",
            ShieldToolApprovalTier::ExplicitConfirmation,
            "High Risk",
        ),
        (
            "mcp_execute_remote_tool",
            ShieldToolApprovalTier::ExplicitConfirmation,
            "High Risk",
        ),
    ] {
        let action = RequestedAction {
            kind: kind.to_string(),
            principal: Some("principal".to_string()),
            path: Some("workspace/output.txt".to_string()),
            content: Some("payload".to_string()),
        };
        let approval = build_shield_approval_request(&action)
            .unwrap_or_else(|| panic!("{kind} should request approval"));
        assert_eq!(approval.approval_tier, expected_tier.as_str());
        assert_eq!(approval.risk_tier, expected_risk);
        assert_eq!(classify_registered_system_tool(kind), Some(expected_tier));
    }
}

#[test]
fn app_session_scope_must_be_offered_by_the_backend() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_scope_kind_validation_{}_{}",
        std::process::id(),
        unix_time_ms_i64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(temp_dir.join("note.txt").display().to_string()),
        content: Some("test".to_string()),
    };
    let mut approval = build_shield_approval_request(&action).unwrap();
    approval.action_type = "codebase_patch".to_string();
    approval.action_class = "codebase_patch".to_string();
    approval.approval_scope_kinds = vec!["once".to_string(), "persistent".to_string()];

    let manager = ScopeTrustManager::default();
    let error = manager
        .grant_from_approval(
            &approval,
            Some(&ScopeTrustApprovalRequest {
                enabled: true,
                duration_ms: Some(DEFAULT_SCOPE_TRUST_DURATION_MS),
                kind: None,
                max_uses: None,
            }),
        )
        .expect_err("an omitted kind must not create hidden non-filesystem trust");
    assert_eq!(error.code, "scope_trust_kind_invalid");
    let error = manager
        .grant_from_approval(
            &approval,
            Some(&ScopeTrustApprovalRequest {
                enabled: true,
                duration_ms: None,
                kind: Some("app_session".to_string()),
                max_uses: None,
            }),
        )
        .expect_err("an unoffered application-session scope must be rejected");
    assert_eq!(error.code, "scope_trust_kind_invalid");
    assert!(manager.session_grants().unwrap().is_empty());

    approval.mandatory_reconfirm = true;
    approval
        .approval_scope_kinds
        .push("app_session".to_string());
    let error = manager
        .grant_from_approval(
            &approval,
            Some(&ScopeTrustApprovalRequest {
                enabled: true,
                duration_ms: None,
                kind: Some("app_session".to_string()),
                max_uses: None,
            }),
        )
        .expect_err("mandatory reconfirmation must never create reusable authority");
    assert_eq!(error.code, "scope_trust_unavailable");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn tool_errors_report_failed_status() {
    let response = ExecuteCommandResponse::from_tool_error(ToolError {
        operation: "file_write".to_string(),
        message: "write failed".to_string(),
    });
    let value = serde_json::to_value(response).expect("response serializes");

    assert_eq!(value["status"], serde_json::json!("failed"));
    assert_eq!(value["verified"], serde_json::json!(false));
}

#[test]
fn deterministic_system_metrics_do_not_claim_a_model_identity() {
    let response = get_system_metrics(SystemMetricsRequest {
        principal: "test-principal".to_string(),
    });
    assert!(response.model_used.is_none());
}

#[test]
fn gateway_message_with_unauthorized_sender_is_dropped() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_gateway_sender_drop_{}", unix_time_ms_i64()));
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .upsert_channel_config(crate::db::SaveChannelConfigRequest {
            platform: "telegram".to_string(),
            is_active: true,
            credentials_json: Some("{\"botToken\":\"redacted\"}".to_string()),
            owner_id: Some("owner-chat-id".to_string()),
        })
        .unwrap();

    let message = GatewayIncomingMessage {
        platform: "telegram".to_string(),
        sender_id: "intruder-chat-id".to_string(),
        sender_display_name: None,
        channel_id: None,
        body: "status".to_string(),
        message_id: Some("m-1".to_string()),
        received_at_ms: unix_time_ms_i64(),
        requested_actions: Vec::new(),
    };
    let decision = verify_gateway_message_allowlist(&persistence, &message)
        .expect("gateway allowlist check succeeds");

    assert!(!decision.allowed);
    assert_eq!(decision.reason, "unauthorized_sender");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn gateway_message_refuses_owner_identity_from_secret_credentials() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_gateway_owner_json_{}", unix_time_ms_i64()));
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .upsert_channel_config(crate::db::SaveChannelConfigRequest {
            platform: "telegram".to_string(),
            is_active: true,
            credentials_json: Some(
                "{\"botToken\":\"redacted\",\"ownerChatId\":\"owner-chat-id\"}".to_string(),
            ),
            owner_id: None,
        })
        .unwrap();

    let message = GatewayIncomingMessage {
        platform: "telegram".to_string(),
        sender_id: "owner-chat-id".to_string(),
        sender_display_name: None,
        channel_id: None,
        body: "status".to_string(),
        message_id: Some("m-1".to_string()),
        received_at_ms: unix_time_ms_i64(),
        requested_actions: Vec::new(),
    };
    let decision = verify_gateway_message_allowlist(&persistence, &message)
        .expect("gateway allowlist check succeeds");

    assert!(!decision.allowed);
    assert_eq!(decision.reason, "owner_unset");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn gateway_remote_shell_action_is_blocked_for_authorized_sender() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_gateway_shell_block_{}", unix_time_ms_i64()));
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .upsert_channel_config(crate::db::SaveChannelConfigRequest {
            platform: "discord".to_string(),
            is_active: true,
            credentials_json: Some("{\"apiKey\":\"redacted\"}".to_string()),
            owner_id: Some("owner-discord-id".to_string()),
        })
        .unwrap();
    let message = GatewayIncomingMessage {
        platform: "discord".to_string(),
        sender_id: "owner-discord-id".to_string(),
        sender_display_name: None,
        channel_id: None,
        body: "rm -rf /".to_string(),
        message_id: Some("m-2".to_string()),
        received_at_ms: unix_time_ms_i64(),
        requested_actions: Vec::new(),
    };

    assert!(
        verify_gateway_message_allowlist(&persistence, &message)
            .expect("gateway allowlist check succeeds")
            .allowed
    );

    let response = filter_gateway_remote_actions(&[RequestedAction {
        kind: "shell_command".to_string(),
        principal: None,
        path: None,
        content: Some("rm -rf /".to_string()),
    }]);

    assert!(response.auto_approved_actions.is_empty());
    assert!(response.confirmation_required_actions.is_empty());
    assert_eq!(response.blocked_actions.len(), 1);
    assert_eq!(
        response.response_message.as_deref(),
        Some(REMOTE_LEVEL_THREE_BLOCK_MESSAGE)
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn authority_boundary_proof_binds_actor_session_scope_and_single_step() {
    let authority = crate::authority::NativeAuthorityManager::default();
    let request = crate::authority::RequestNativeAuthorityProof {
        session_id: "session-authority-test".to_string(),
        operation_classes: vec!["filesystem_write".to_string()],
        scopes: vec!["actuation-session:session-authority-test".to_string()],
        max_steps: 1,
        persistence: "one_time".to_string(),
        locale: None,
    };
    let missing = authority.consume(
        "fabricated",
        crate::authority::NativeAuthorityExpectation {
            actor_id: "actor-test".to_string(),
            session_id: request.session_id.clone(),
            operation_classes: request.operation_classes.clone(),
            canonical_scopes: request.scopes.clone(),
            max_steps: 1,
            allowed_persistences: vec!["one_time".to_string()],
        },
    );
    assert_eq!(missing.unwrap_err().code, "authority_proof_missing");

    let proof = authority
        .issue_test_harness("actor-test".to_string(), request.clone())
        .unwrap();
    authority
        .consume(
            &proof.proof_id,
            crate::authority::NativeAuthorityExpectation {
                actor_id: "actor-test".to_string(),
                session_id: request.session_id.clone(),
                operation_classes: request.operation_classes.clone(),
                canonical_scopes: request.scopes.clone(),
                max_steps: 1,
                allowed_persistences: vec!["one_time".to_string()],
            },
        )
        .unwrap();
    assert_eq!(
        authority
            .consume(
                &proof.proof_id,
                crate::authority::NativeAuthorityExpectation {
                    actor_id: "actor-test".to_string(),
                    session_id: request.session_id.clone(),
                    operation_classes: request.operation_classes.clone(),
                    canonical_scopes: request.scopes.clone(),
                    max_steps: 1,
                    allowed_persistences: vec!["one_time".to_string()],
                },
            )
            .unwrap_err()
            .code,
        "authority_proof_missing"
    );

    let leases = ActuationLeaseManager::default();
    leases
        .grant(
            "actor-test".to_string(),
            &request.session_id,
            request.operation_classes,
            request.scopes,
            60_000,
            1,
        )
        .unwrap();
    let action = AuthorizedActions::FileWrite(FileWriteRequest {
        path: "workspace/authority-bound.txt".to_string(),
        content: "verified".to_string(),
    });
    assert!(matches!(
        leases
            .evaluate_autonomous_action(
                None,
                Some("different-actor"),
                Some("session-authority-test"),
                &action,
            )
            .unwrap(),
        ActuationLeaseOutcome::Blocked(_, _)
    ));
    assert!(matches!(
        leases
            .evaluate_autonomous_action(
                None,
                Some("actor-test"),
                Some("session-authority-test"),
                &action,
            )
            .unwrap(),
        ActuationLeaseOutcome::Authorized(_)
    ));
    assert!(matches!(
        leases
            .evaluate_autonomous_action(
                None,
                Some("actor-test"),
                Some("session-authority-test"),
                &action,
            )
            .unwrap(),
        ActuationLeaseOutcome::Blocked(_, _)
    ));
}

#[test]
fn sovereign_duckduckgo_search_is_low_risk_allowlisted() {
    let action = RequestedAction {
        kind: "sovereign_duckduckgo_search".to_string(),
        principal: Some("Red Sox score today".to_string()),
        path: None,
        content: Some("12".to_string()),
    };

    match authorize_action(action).expect("search is allowlisted") {
        AuthorizedActions::SovereignDuckDuckGoSearch(request) => {
            assert_eq!(request.query, "Red Sox score today");
            assert_eq!(request.max_results, 5);
        }
        other => panic!("expected sovereign search, got {other:?}"),
    }
}

#[test]
fn test_pii_healthcare_masking_regression() {
    let healthcare_data = "Patient: John Doe, DOB: 1985-05-12, SSN: 123-45-6789. Admitted to General Hospital with MRN-99812. Address: 123 Main Street, Boston, MA 02110. Contact: 617-555-0199 or email john.doe@example.com. IP logged: 192.168.1.105.";
    let masked = mask_pii(healthcare_data);

    // Ensure no sensitive raw values leak
    assert!(
        !masked.contains("John Doe"),
        "Raw name John Doe leaked: {}",
        masked
    );
    assert!(!masked.contains("1985-05-12"), "Raw DOB leaked: {}", masked);
    assert!(
        !masked.contains("123-45-6789"),
        "Raw SSN leaked: {}",
        masked
    );
    assert!(!masked.contains("MRN-99812"), "Raw MRN leaked: {}", masked);
    assert!(
        !masked.contains("123 Main Street"),
        "Raw address leaked: {}",
        masked
    );
    assert!(
        !masked.contains("617-555-0199"),
        "Raw phone leaked: {}",
        masked
    );
    assert!(
        !masked.contains("john.doe@example.com"),
        "Raw email leaked: {}",
        masked
    );
    assert!(
        !masked.contains("192.168.1.105"),
        "Raw IP leaked: {}",
        masked
    );

    // Ensure PII placeholders are injected
    assert!(masked.contains("{{PII_MASKED}}"));
}
