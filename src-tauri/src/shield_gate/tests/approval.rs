use super::*;

#[test]
fn channel_approval_preview_never_contains_credentials() {
    let action = RequestedAction {
        kind: "configure_channel".to_string(),
        principal: None,
        path: None,
        content: Some(
            serde_json::json!({
                "platform": "telegram",
                "credentials_json": "{\"botToken\":\"secret-token-canary\"}",
                "owner_id": "42",
                "is_active": true,
            })
            .to_string(),
        ),
    };

    let preview = approval_preview(&action);
    assert!(preview.contains("telegram"));
    assert!(preview.contains("42"));
    assert!(preview.contains("credentialsProvided"));
    assert!(!preview.contains("secret-token-canary"));
    assert_eq!(
        reviewed_action_class("configure_channel"),
        "filesystem_write"
    );
}

#[test]
fn native_action_approval_preview_preserves_complete_bounded_json() {
    let body = "decision evidence ".repeat(80);
    let content = serde_json::json!({
        "to": "reviewer@example.com",
        "subject": "Supplier Decision Review",
        "body": body,
    })
    .to_string();
    assert!(content.len() > 700);
    let action = RequestedAction {
        kind: "draft_system_email".to_string(),
        principal: None,
        path: None,
        content: Some(content.clone()),
    };

    let preview = approval_preview(&action);
    assert_eq!(preview, content);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&preview).unwrap()["body"],
        body
    );
}

#[test]
fn read_only_system_tools_bypass_manual_approval_requests() {
    for kind in [
        "file_read",
        "file_list",
        "get_system_metrics",
        "codebase_compile",
    ] {
        let action = RequestedAction {
            kind: kind.to_string(),
            principal: Some("principal".to_string()),
            path: Some("workspace/readme.md".to_string()),
            content: None,
        };
        assert_eq!(
            classify_registered_system_tool(kind),
            Some(ShieldToolApprovalTier::BackgroundAutoApproval)
        );
        assert!(
            build_shield_approval_request(&action).is_none(),
            "{kind} should not open a manual approval prompt"
        );
    }
}

#[test]
fn direct_user_commands_use_shield_approval_without_certificate_precheck() {
    assert!(!requires_logical_certificate("file_write"));
    assert!(!requires_logical_certificate("delete_file"));
    assert!(!requires_logical_certificate("shell_command"));
    assert_eq!(
        classify_registered_system_tool("file_write"),
        Some(ShieldToolApprovalTier::VisualConsent)
    );
    assert_eq!(
        classify_registered_system_tool("shell_command"),
        Some(ShieldToolApprovalTier::ExplicitConfirmation)
    );
}

#[test]
fn scope_trust_cache_allows_matching_action_until_expiration() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_scope_trust_{}", unix_time_ms_i64()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let trusted_action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(temp_dir.join("notes.md").display().to_string()),
        content: Some("trusted".to_string()),
    };
    let approval = build_shield_approval_request(&trusted_action)
        .expect("file write should request visual consent");
    let scope_trust = ScopeTrustManager::default();

    assert!(scope_trust
        .grant_from_approval(
            &approval,
            Some(&ScopeTrustApprovalRequest {
                enabled: true,
                duration_ms: Some(1_000),
                kind: None,
                max_uses: None,
            }),
        )
        .expect("scope trust should be granted"));
    assert!(scope_trust
        .allows_action(&trusted_action)
        .expect("trusted action should be checked"));

    let outside_action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(
            temp_dir
                .parent()
                .unwrap()
                .join("outside-scope.md")
                .display()
                .to_string(),
        ),
        content: Some("outside".to_string()),
    };
    assert!(!scope_trust
        .allows_action(&outside_action)
        .expect("outside action should be checked"));

    std::thread::sleep(Duration::from_secs(2));
    assert!(!scope_trust
        .allows_action(&trusted_action)
        .expect("expired action should be checked"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn shield_native_decision_status_exposes_no_authorizing_token() {
    tauri::async_runtime::block_on(async {
        let action = RequestedAction {
            kind: "shell_command".to_string(),
            principal: Some("local_principal".to_string()),
            path: None,
            content: Some("printf canary".to_string()),
        };
        let mut approval = build_shield_approval_request(&action).unwrap();
        approval.session_id = Some("session-1".to_string());
        approval.turn_id = Some("turn-1".to_string());
        approval.generation_token = Some("generation-1".to_string());
        let frozen = crate::authority::shield_decision::freeze_request(&approval).unwrap();
        let secret = approval.approval_token.clone();
        let manager = ShieldApprovalManager::default();
        manager.pending.lock().await.insert(
            secret.clone(),
            PendingShieldApproval {
                request: approval,
                frozen,
                display_id: "shieldstatus-display-only".to_string(),
            },
        );

        let projection = manager.pending_requests().await;
        assert_eq!(projection.len(), 1);
        assert_eq!(projection[0].display_id, "shieldstatus-display-only");
        let serialized = serde_json::to_string(&projection).unwrap();
        assert!(!serialized.contains(&secret));
        assert!(!serialized.contains("approvalToken"));
    });
}

#[test]
fn sovereign_trust_claim_labels_estimates_and_reservations() {
    let claim = sovereign_trust_reservation_claim(
        "global_trust",
        "/tmp/trusted",
        "~/trusted",
        17,
        0.250,
        0.012,
    );
    assert!(claim.contains("estimated_token_cost=17"));
    assert!(claim.contains("reserved_cpu_seconds=0.250"));
    assert!(claim.contains("observed_elapsed_wall_seconds=0.012"));
    assert!(!claim.contains(" token_cost="));
    assert!(!claim.contains(" cpu_seconds="));
}

#[test]
fn approved_shell_command_verifies_only_zero_exit() {
    let working_directory = std::env::current_dir()
        .expect("test current directory resolves")
        .display()
        .to_string();
    let success = handle_authorized_action(AuthorizedActions::ApprovedSystemExecution(
        SystemExecutionRequest {
            executable: "printf".to_string(),
            args: vec!["ok".to_string()],
            env: std::collections::BTreeMap::new(),
            cwd: Some(working_directory.clone()),
            timeout: None,
        },
    ));
    let success_json = serde_json::to_value(success).expect("success serializes");
    assert_eq!(success_json["status"], serde_json::json!("completed"));
    assert_eq!(success_json["verified"], serde_json::json!(true));

    let failure = handle_authorized_action(AuthorizedActions::ApprovedSystemExecution(
        SystemExecutionRequest {
            executable: "sh".to_string(),
            args: vec!["-c".to_string(), "exit 7".to_string()],
            env: std::collections::BTreeMap::new(),
            cwd: Some(working_directory),
            timeout: None,
        },
    ));
    let failure_json = serde_json::to_value(failure).expect("failure serializes");
    assert_eq!(failure_json["status"], serde_json::json!("failed"));
    assert_eq!(failure_json["verified"], serde_json::json!(false));
}

#[test]
fn global_trust_policy_silently_authorizes_external_write_scope() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_shield_global_trust_{}", unix_time_ms_i64()));
    let trusted_dir = temp_dir.join("trusted");
    std::fs::create_dir_all(&trusted_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .upsert_sovereign_trust_policy(
            trusted_dir.to_str().unwrap(),
            &[SovereignTrustToolCategory::ExternalWrites],
            SovereignTrustPermissionLevel::GlobalTrust,
            None,
            Some(256),
            Some(2.0),
        )
        .unwrap();

    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(trusted_dir.join("notes.md").display().to_string()),
        content: Some("trusted write".to_string()),
    };
    let decision = evaluate_sovereign_trust_for_action(&persistence, &action, None)
        .expect("trust evaluation should succeed");
    let trusted = match decision {
        SovereignTrustDecision::Trusted(trusted) => trusted,
        SovereignTrustDecision::PromptRequired => panic!("global policy should auto trust"),
    };
    assert_eq!(
        trusted.grant.permission_level,
        SovereignTrustPermissionLevel::GlobalTrust
    );

    let context = ShieldAuthorizationContext {
        shield_approved: true,
        trusted_working_directory: Some(trusted.grant.canonical_directory_path),
    };
    match authorize_action_for_execution_with_context(action, &context)
        .expect("trusted write should authorize")
    {
        AuthorizedActions::ApprovedExternalFileWrite(request) => {
            assert!(request.path.ends_with("notes.md"));
        }
        other => panic!("expected trusted external write, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn session_trust_policy_silently_authorizes_matching_chat_session() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_shield_session_trust_{}", unix_time_ms_i64()));
    let trusted_dir = temp_dir.join("strategy");
    std::fs::create_dir_all(&trusted_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .activate_sovereign_trust_session(
            "chat-strategy",
            trusted_dir.to_str().unwrap(),
            &[SovereignTrustToolCategory::ExternalWrites],
            Some(unix_time_ms_i64() + crate::db::SOVEREIGN_TRUST_SESSION_DURATION_MS),
            None,
            None,
        )
        .unwrap();

    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(trusted_dir.join("sprint.md").display().to_string()),
        content: Some("session write".to_string()),
    };
    assert!(matches!(
        evaluate_sovereign_trust_for_action(&persistence, &action, Some("chat-strategy")).unwrap(),
        SovereignTrustDecision::Trusted(_)
    ));
    assert!(matches!(
        evaluate_sovereign_trust_for_action(&persistence, &action, Some("wrong-session")).unwrap(),
        SovereignTrustDecision::PromptRequired
    ));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn trusted_shell_command_uses_grant_scope_as_working_directory() {
    let trusted_dir =
        std::env::temp_dir().join(format!("oomu_trusted_shell_scope_{}", unix_time_ms_i64()));
    std::fs::create_dir_all(&trusted_dir).unwrap();
    let action = RequestedAction {
        kind: "shell_command".to_string(),
        principal: None,
        path: None,
        content: Some("pwd".to_string()),
    };
    let context = ShieldAuthorizationContext {
        shield_approved: true,
        trusted_working_directory: Some(trusted_dir.display().to_string()),
    };

    match authorize_action_for_execution_with_context(action, &context)
        .expect("trusted shell should authorize")
    {
        AuthorizedActions::ApprovedSystemExecution(request) => {
            assert_eq!(request.cwd.as_deref(), Some(trusted_dir.to_str().unwrap()));
        }
        other => panic!("expected approved system execution, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(trusted_dir);
}

#[test]
fn actuation_lease_allows_three_mutating_steps_then_decays() {
    let leases = ActuationLeaseManager::default();
    let status = leases
        .grant(
            "actor-test".to_string(),
            "session-lease-test",
            vec!["filesystem_write".to_string()],
            vec!["actuation-session:session-lease-test".to_string()],
            5 * 60 * 1_000,
            3,
        )
        .expect("lease is granted");
    assert!(status.active);
    assert_eq!(status.remaining_steps, 3);

    let action = AuthorizedActions::FileWrite(FileWriteRequest {
        path: "workspace/lease-test.txt".to_string(),
        content: "step".to_string(),
    });

    for expected_steps in 1..=3 {
        match leases
            .evaluate_autonomous_action(
                None,
                Some("actor-test"),
                Some("session-lease-test"),
                &action,
            )
            .expect("lease check succeeds")
        {
            ActuationLeaseOutcome::Authorized(status) => {
                assert_eq!(
                    status.lease.as_ref().map(|lease| lease.current_steps),
                    Some(expected_steps)
                );
                assert_eq!(status.remaining_steps, 3 - expected_steps);
            }
            other => panic!("expected authorized lease step, got {other:?}"),
        }
    }

    match leases
        .evaluate_autonomous_action(
            None,
            Some("actor-test"),
            Some("session-lease-test"),
            &action,
        )
        .expect("lease check succeeds")
    {
        ActuationLeaseOutcome::Blocked(status, reason) => {
            assert!(!status.active);
            assert!(reason.contains("expired") || reason.contains("step budget"));
            assert_eq!(
                status.lease.as_ref().map(|lease| lease.is_active),
                Some(false)
            );
        }
        other => panic!("expected exhausted lease to block, got {other:?}"),
    }
}

#[test]
fn actuation_lease_blocks_mutating_action_after_timeout() {
    let leases = ActuationLeaseManager::default();
    let status = leases
        .grant(
            "actor-test".to_string(),
            "session-timeout-test",
            vec!["filesystem_write".to_string()],
            vec!["actuation-session:session-timeout-test".to_string()],
            1_000,
            3,
        )
        .expect("lease is granted");
    assert!(status.active);

    std::thread::sleep(Duration::from_secs(2));

    let action = AuthorizedActions::FileWrite(FileWriteRequest {
        path: "workspace/lease-timeout.txt".to_string(),
        content: "step".to_string(),
    });

    match leases
        .evaluate_autonomous_action(
            None,
            Some("actor-test"),
            Some("session-timeout-test"),
            &action,
        )
        .expect("lease check succeeds")
    {
        ActuationLeaseOutcome::Blocked(status, _) => {
            assert!(!status.active);
            assert_eq!(
                status.lease.as_ref().map(|lease| lease.is_active),
                Some(false)
            );
        }
        other => panic!("expected timed out lease to block, got {other:?}"),
    }
}

#[test]
fn finishing_agent_session_revokes_only_its_exact_session_lease() {
    let leases = ActuationLeaseManager::default();
    leases
        .grant(
            "actor-test".to_string(),
            "session-a",
            vec!["filesystem_write".to_string()],
            vec!["actuation-session:session-a".to_string()],
            5 * 60 * 1_000,
            2,
        )
        .expect("session lease is granted");

    assert!(leases
        .finish_session(None, Some("session-b"), "agent_execution_completed")
        .is_none());
    assert!(leases.snapshot().active);

    let finished = leases
        .finish_session(None, Some("session-a"), "agent_execution_completed")
        .expect("matching session lease is finished");
    assert!(!finished.active);
    assert_eq!(
        finished.reason.as_deref(),
        Some("agent_execution_completed")
    );

    assert_eq!(
        required_actuation_session_id(None).unwrap_err().code,
        "actuation_session_required"
    );
}

#[test]
fn lease_management_commands_are_not_model_routable_actions() {
    for kind in [
        "get_actuation_lease_status",
        "grant_actuation_lease",
        "revoke_actuation_lease",
    ] {
        let rejected = authorize_action(RequestedAction {
            kind: kind.to_string(),
            principal: None,
            path: None,
            content: None,
        })
        .expect_err("lease management command must not be model-routable");
        assert_eq!(rejected.code, "shield_gate_rejected");
    }
}

#[test]
fn sovereign_trust_blocks_when_daily_token_limit_is_exceeded() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_shield_trust_limit_{}", unix_time_ms_i64()));
    let trusted_dir = temp_dir.join("trusted");
    std::fs::create_dir_all(&trusted_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .upsert_sovereign_trust_policy(
            trusted_dir.to_str().unwrap(),
            &[SovereignTrustToolCategory::ExternalWrites],
            SovereignTrustPermissionLevel::GlobalTrust,
            None,
            Some(1),
            Some(2.0),
        )
        .unwrap();

    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(trusted_dir.join("too-large.md").display().to_string()),
        content: Some("this payload is larger than one token".to_string()),
    };
    let rejected = evaluate_sovereign_trust_for_action(&persistence, &action, None)
        .expect_err("trusted action must stop at non-bypassable token limit");
    assert_eq!(rejected.code, "sovereign_trust_resource_limit_exceeded");
    assert!(rejected
        .message
        .contains("estimated token-cost reservation limit"));
    assert!(rejected.message.contains("requested estimate"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sovereign_trust_cpu_quota_is_labeled_as_a_reservation() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_shield_cpu_reservation_{}",
        unix_time_ms_i64()
    ));
    let trusted_dir = temp_dir.join("trusted");
    std::fs::create_dir_all(&trusted_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .upsert_sovereign_trust_policy(
            trusted_dir.to_str().unwrap(),
            &[SovereignTrustToolCategory::ExternalWrites],
            SovereignTrustPermissionLevel::GlobalTrust,
            None,
            Some(10_000),
            Some(0.01),
        )
        .unwrap();

    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(trusted_dir.join("note.md").display().to_string()),
        content: Some("observed content".to_string()),
    };
    let rejected = evaluate_sovereign_trust_for_action(&persistence, &action, None)
        .expect_err("CPU reservation estimate must respect the quota");
    assert_eq!(rejected.code, "sovereign_trust_resource_limit_exceeded");
    assert!(rejected.message.contains("CPU-seconds reservation limit"));
    assert!(rejected.message.contains("requested estimate"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn shell_command_requires_shield_approval() {
    let action = RequestedAction {
        kind: "shell_command".to_string(),
        principal: None,
        path: None,
        content: Some("echo shield-ok".to_string()),
    };

    let rejected = authorize_action(action.clone()).expect_err("shell command must gate");
    assert_eq!(rejected.code, "shield_gate_rejected");

    match authorize_action_for_execution(action, true).expect("approved shell is classified") {
        AuthorizedActions::ApprovedSystemExecution(request) => {
            assert_eq!(request.executable, "echo");
            assert_eq!(request.args, ["shield-ok"]);
        }
        other => panic!("expected approved shell command, got {other:?}"),
    }
}

#[test]
fn typed_terminal_reads_gate_only_when_they_leave_the_project() {
    let project = development_repo_root()
        .canonicalize()
        .expect("development project resolves");
    let project_read = RequestedAction {
        kind: "terminal_execute".to_string(),
        principal: None,
        path: None,
        content: Some(
            serde_json::json!({
                "executable": "git",
                "args": ["status", "--short"],
                "env": {},
                "cwd": project,
                "timeout": 30_000
            })
            .to_string(),
        ),
    };
    assert!(build_shield_approval_request(&project_read).is_none());
    assert!(matches!(
        authorize_action(project_read),
        Ok(AuthorizedActions::ApprovedSystemExecution(_))
    ));

    let external_dir = std::env::temp_dir().join(format!(
        "oomu-terminal-external-read-{}",
        unix_time_ms_i64()
    ));
    std::fs::create_dir_all(&external_dir).unwrap();
    let external_file = external_dir.join("attached-plan.md");
    std::fs::write(&external_file, "Sprint 299 acceptance plan").unwrap();
    let external_read = RequestedAction {
        kind: "terminal_execute".to_string(),
        principal: None,
        path: None,
        content: Some(
            serde_json::json!({
                "executable": "rg",
                "args": ["Sprint 299", external_file],
                "env": {},
                "cwd": project,
                "timeout": 30_000
            })
            .to_string(),
        ),
    };
    let approval = build_shield_approval_request(&external_read)
        .expect("external terminal read requires folder consent");
    assert_eq!(approval.action_class, "filesystem_read");
    assert!(approval.scope_trust_available);
    assert!(approval
        .approval_scope_kinds
        .contains(&"app_session".to_string()));
    assert_eq!(
        authorize_action(external_read).unwrap_err().code,
        "shield_gate_rejected"
    );
    let _ = std::fs::remove_dir_all(external_dir);
}
