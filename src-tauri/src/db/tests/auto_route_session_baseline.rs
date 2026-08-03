use super::*;

const TEST_LOCAL_PROVIDER_CONFIG_ID: &str = "prov-local-auto-route-test";

fn verified_test_baseline(model_id: &str) -> VerifiedAutoRouteBaseline {
    VerifiedAutoRouteBaseline {
        provider_config_id: ProviderConfigurationId::try_from(
            TEST_LOCAL_PROVIDER_CONFIG_ID.to_string(),
        )
        .expect("test provider configuration ID"),
        provider_type: ProviderTypeId::try_from("local_model".to_string())
            .expect("test provider type"),
        model_id: CanonicalModelId::try_from(model_id.to_string()).expect("test model ID"),
        reasoning_depth: "medium".to_string(),
        context_budget: 16_384,
        provenance: AutoRouteProvenance::ExplicitSession,
    }
}

#[test]
fn session_without_a_baseline_reads_as_generation_zero() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-no-baseline-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-no-baseline".to_string(),
            provider_id: "local_model".to_string(),
            model_id: crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            title: None,
            dynamic_routing_override: Some(false),
            workspace_id: None,
        })
        .expect("manual session is created");

    let policy = engine
        .select_chat_session_route_policy(&session.id)
        .expect("policy read accepts a missing optional baseline")
        .expect("session policy exists");
    assert_eq!(policy.route_generation, 0);
    assert_eq!(policy.local_provider_id, None);
    assert_eq!(policy.local_provider_type, None);

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn dynamic_session_and_local_baseline_commit_together() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-baseline-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");

    let session = engine
        .ensure_chat_session_with_auto_route_baseline(
            CreateChatSessionRequest {
                agent_id: "agent-test".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Auto-route".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            verified_test_baseline("gemma-4-12B-it-qat-q4_0-gguf"),
            &installed_model_root(),
        )
        .expect("session and baseline commit");

    let policy = engine
        .select_chat_session_route_policy(&session.id)
        .expect("policy read succeeds")
        .expect("policy exists");
    assert_eq!(policy.session_provider_id, "dynamic");
    assert_eq!(policy.session_model_id, "dynamic");
    assert_eq!(
        policy.local_provider_id.as_deref(),
        Some(TEST_LOCAL_PROVIDER_CONFIG_ID)
    );
    assert_eq!(policy.local_provider_type.as_deref(), Some("local_model"));
    assert_eq!(policy.route_generation, 1);
    assert_eq!(
        policy.local_model_id.as_deref(),
        Some("gemma-4-12B-it-qat-q4_0-gguf")
    );
    assert_eq!(policy.context_budget, Some(16_384));
    let config = engine.select_session_config(&session.id).unwrap().unwrap();
    assert_eq!(
        config.local_provider_config_id.as_deref(),
        Some(TEST_LOCAL_PROVIDER_CONFIG_ID)
    );
    assert_eq!(config.local_provider_type.as_deref(), Some("local_model"));

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn partial_typed_dynamic_identity_never_falls_back_to_legacy_provider_column() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-partial-provider-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).unwrap();
    let session = engine
        .ensure_chat_session_with_auto_route_baseline(
            CreateChatSessionRequest {
                agent_id: "agent-partial-provider".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: None,
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            verified_test_baseline(crate::gemma::GEMMA_E4B_CANONICAL_ID),
            &installed_model_root(),
        )
        .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE active_session_configs SET local_provider_type=NULL WHERE session_id=?1",
            params![session.id],
        )
        .unwrap();
    let policy = engine
        .select_chat_session_route_policy(&session.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        policy.local_provider_id.as_deref(),
        Some(TEST_LOCAL_PROVIDER_CONFIG_ID)
    );
    assert_eq!(policy.local_provider_type, None);
    let queued = engine.insert_queued_message(QueueMessageRequest {
        turn_id: Some("turn-partial-provider".to_string()),
        generation_token: Some("generation-partial-provider".to_string()),
        parent_turn_id: None,
        root_turn_id: Some("turn-partial-provider".to_string()),
        turn_kind: Some("root".to_string()),
        agent_id: session.agent_id.clone(),
        message: "Do not use a partial identity.".to_string(),
        attachments: Vec::new(),
        session_id: Some(session.id.clone()),
        provider_id: Some("dynamic".to_string()),
        model_id: Some("dynamic".to_string()),
        reasoning: Some("medium".to_string()),
        context: Some("16384".to_string()),
        context_budget: Some(16_384),
        steering: None,
        automated_web_grounding_enabled: Some(false),
        dynamic_routing_override: Some(true),
    });
    assert!(queued.is_err());
    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn accepted_root_turn_keeps_its_frozen_policy_after_reload_and_session_edit() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-frozen-turn-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let session = engine
        .ensure_chat_session_with_auto_route_baseline(
            CreateChatSessionRequest {
                agent_id: "agent-test".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Frozen Auto-route".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            verified_test_baseline("gemma-4-12B-it-qat-q4_0-gguf"),
            &installed_model_root(),
        )
        .expect("session and baseline commit");
    engine
        .accept_chat_turn(AcceptChatTurnRequest {
            turn_id: "turn-auto-route-frozen".to_string(),
            generation_token: "generation-auto-route-frozen".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-auto-route-frozen".to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: "agent-test".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            message: "Hello OOMU".to_string(),
        })
        .expect("turn accepted");
    let first = AutoRouteTurnPolicyRecord {
        local_provider_id: TEST_LOCAL_PROVIDER_CONFIG_ID.to_string(),
        local_provider_type: "local_model".to_string(),
        local_model_id: "gemma-4-12B-it-qat-q4_0-gguf".to_string(),
        local_reasoning: "medium".to_string(),
        local_context_budget: 16_384,
        local_source: "explicit_session".to_string(),
        route_generation: 1,
        cloud_provider_id: Some("prov-cloud".to_string()),
        cloud_model_id: Some("gemini-3.5-flash".to_string()),
        cloud_provider_name: Some("Gemini".to_string()),
        classifier_model_id: Some("gemma-4-E2B-it-qat-q4_0-gguf".to_string()),
        classifier_version: "local_difficulty_v2".to_string(),
        policy_version: "auto_route_policy_v2".to_string(),
        frozen_at_ms: 1,
    };
    let frozen = engine
        .freeze_auto_route_turn_policy(
            "turn-auto-route-frozen",
            "generation-auto-route-frozen",
            &session.id,
            "agent-test",
            first.clone(),
        )
        .expect("policy freezes");
    assert_eq!(frozen, first);

    drop(engine);
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database reloads");
    engine
        .open_connection()
        .expect("database writes")
        .execute(
            "UPDATE active_session_configs
             SET local_provider_config_id = 'replacement-config',
                 local_provider_type = 'replacement-provider',
                 model_id = 'replacement-model',
                 local_model_source = 'startup_default',
                 local_route_generation = local_route_generation + 1
             WHERE session_id = ?1",
            params![&session.id],
        )
        .expect("session route changes after the turn is frozen");

    let changed_candidate = AutoRouteTurnPolicyRecord {
        local_model_id: "gemma-4-E4B-it-qat-q4_0-gguf".to_string(),
        cloud_model_id: Some("different-cloud-model".to_string()),
        frozen_at_ms: 2,
        ..first.clone()
    };
    let retry = engine
        .freeze_auto_route_turn_policy(
            "turn-auto-route-frozen",
            "generation-auto-route-frozen",
            &session.id,
            "agent-test",
            changed_candidate,
        )
        .expect("retry after reload reads the original frozen policy");
    assert_eq!(retry, first);

    drop(engine);
    let _ = std::fs::remove_file(path);
}

fn accept_and_freeze_unavailable_explicit_turn(
    engine: &PersistenceEngine,
    session: &ChatSessionRecord,
    turn_id: &str,
    generation_token: &str,
) -> AutoRouteTurnPolicyRecord {
    engine
        .open_connection()
        .expect("database writes")
        .execute(
            "UPDATE active_session_configs
             SET provider_id = 'local_model', model_id = 'missing-explicit-model',
                 local_model_source = 'explicit_session'
             WHERE session_id = ?1",
            params![&session.id],
        )
        .expect("unavailable explicit baseline saves");
    engine
        .accept_chat_turn(AcceptChatTurnRequest {
            turn_id: turn_id.to_string(),
            generation_token: generation_token.to_string(),
            parent_turn_id: None,
            root_turn_id: turn_id.to_string(),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            message: "Use my saved on-device model".to_string(),
        })
        .expect("turn accepted");
    let policy = AutoRouteTurnPolicyRecord {
        local_provider_id: TEST_LOCAL_PROVIDER_CONFIG_ID.to_string(),
        local_provider_type: "local_model".to_string(),
        local_model_id: "missing-explicit-model".to_string(),
        local_reasoning: "medium".to_string(),
        local_context_budget: 12_288,
        local_source: "explicit_session".to_string(),
        route_generation: 1,
        cloud_provider_id: Some("prov-cloud".to_string()),
        cloud_model_id: Some("gemini-3.5-flash".to_string()),
        cloud_provider_name: Some("Gemini".to_string()),
        classifier_model_id: Some(crate::gemma::GEMMA_E2B_CANONICAL_ID.to_string()),
        classifier_version: "local_difficulty_v2".to_string(),
        policy_version: "auto_route_policy_v2".to_string(),
        frozen_at_ms: 41,
    };
    engine
        .freeze_auto_route_turn_policy(
            turn_id,
            generation_token,
            &session.id,
            &session.agent_id,
            policy.clone(),
        )
        .expect("unavailable explicit policy freezes");
    policy
}

#[test]
fn explicit_repair_rebinds_only_the_exact_frozen_turn_once() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-exact-repair-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let session = dynamic_session(
        &engine,
        "agent-exact-repair",
        crate::gemma::GEMMA_E2B_CANONICAL_ID,
    );
    let original = accept_and_freeze_unavailable_explicit_turn(
        &engine,
        &session,
        "turn-exact-repair",
        "generation-exact-repair",
    );

    let repaired = engine
        .repair_auto_route_session_baseline(
            &session.id,
            "turn-exact-repair",
            "generation-exact-repair",
            TEST_LOCAL_PROVIDER_CONFIG_ID,
            "local_model",
            crate::gemma::GEMMA_E2B_CANONICAL_ID,
            &installed_model_root(),
        )
        .expect("exact unavailable turn repairs");
    assert_eq!(
        repaired.local_model_id.as_deref(),
        Some(crate::gemma::GEMMA_E2B_CANONICAL_ID)
    );

    let frozen = engine
        .freeze_auto_route_turn_policy(
            "turn-exact-repair",
            "generation-exact-repair",
            &session.id,
            &session.agent_id,
            original.clone(),
        )
        .expect("retry reads the repaired frozen policy");
    assert_eq!(frozen.local_model_id, crate::gemma::GEMMA_E2B_CANONICAL_ID);
    assert_eq!(frozen.cloud_model_id, original.cloud_model_id);
    assert_eq!(frozen.frozen_at_ms, original.frozen_at_ms);

    engine
        .repair_auto_route_session_baseline(
            &session.id,
            "turn-exact-repair",
            "generation-exact-repair",
            TEST_LOCAL_PROVIDER_CONFIG_ID,
            "local_model",
            crate::gemma::GEMMA_E2B_CANONICAL_ID,
            &installed_model_root(),
        )
        .expect("duplicate delivery is idempotent");
    let second_change = engine.repair_auto_route_session_baseline(
        &session.id,
        "turn-exact-repair",
        "generation-exact-repair",
        TEST_LOCAL_PROVIDER_CONFIG_ID,
        "local_model",
        crate::gemma::GEMMA_E4B_CANONICAL_ID,
        &installed_model_root(),
    );
    assert!(second_change.is_err());
    let metadata: String = engine
        .open_connection()
        .expect("database reads")
        .query_row(
            "SELECT metadata_json FROM chat_messages
             WHERE session_id = ?1 AND role = 'user'
               AND json_extract(metadata_json, '$.turnId') = 'turn-exact-repair'",
            params![&session.id],
            |row| row.get(0),
        )
        .expect("repair receipt reads");
    let metadata: Value = serde_json::from_str(&metadata).expect("metadata parses");
    assert_eq!(
        metadata["autoRoutePolicyRepair"]["reason"],
        "unavailable_explicit_session"
    );
    assert_eq!(
        metadata["autoRoutePolicy"]["localModelId"],
        crate::gemma::GEMMA_E2B_CANONICAL_ID
    );

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn explicit_repair_rejects_a_different_saved_turn_identity_atomically() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-wrong-repair-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let session = dynamic_session(
        &engine,
        "agent-wrong-repair",
        crate::gemma::GEMMA_E2B_CANONICAL_ID,
    );
    let original = accept_and_freeze_unavailable_explicit_turn(
        &engine,
        &session,
        "turn-wrong-repair",
        "generation-wrong-repair",
    );

    let result = engine.repair_auto_route_session_baseline(
        &session.id,
        "turn-wrong-repair",
        "different-generation",
        TEST_LOCAL_PROVIDER_CONFIG_ID,
        "local_model",
        crate::gemma::GEMMA_E2B_CANONICAL_ID,
        &installed_model_root(),
    );
    assert!(result.is_err());
    let baseline = engine
        .select_chat_session_route_policy(&session.id)
        .expect("baseline reads")
        .expect("baseline exists");
    assert_eq!(
        baseline.local_model_id.as_deref(),
        Some("missing-explicit-model")
    );
    let frozen = engine
        .freeze_auto_route_turn_policy(
            "turn-wrong-repair",
            "generation-wrong-repair",
            &session.id,
            &session.agent_id,
            AutoRouteTurnPolicyRecord {
                local_model_id: crate::gemma::GEMMA_E2B_CANONICAL_ID.to_string(),
                ..original.clone()
            },
        )
        .expect("failed repair leaves original frozen policy");
    assert_eq!(frozen, original);

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn idle_auto_route_queue_freezes_the_saved_local_baseline() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-queued-baseline-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let session = engine
        .ensure_chat_session_with_auto_route_baseline(
            CreateChatSessionRequest {
                agent_id: "agent-test".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Queued Auto-route".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            verified_test_baseline("gemma-4-12B-it-qat-q4_0-gguf"),
            &installed_model_root(),
        )
        .expect("session and baseline commit");
    let queued = engine
        .insert_queued_message(QueueMessageRequest {
            turn_id: Some("turn-auto-route-queued".to_string()),
            generation_token: Some("generation-auto-route-queued".to_string()),
            parent_turn_id: None,
            root_turn_id: Some("turn-auto-route-queued".to_string()),
            turn_kind: Some("root".to_string()),
            agent_id: "agent-test".to_string(),
            message: "Hello OOMU".to_string(),
            attachments: Vec::new(),
            session_id: Some(session.id),
            provider_id: Some("dynamic".to_string()),
            model_id: Some("dynamic".to_string()),
            reasoning: Some("medium".to_string()),
            context: Some("16384".to_string()),
            context_budget: Some(16_384),
            steering: None,
            automated_web_grounding_enabled: Some(false),
            dynamic_routing_override: Some(true),
        })
        .expect("queued Auto-route turn freezes baseline");
    assert_eq!(
        queued.provider_id.as_deref(),
        Some(TEST_LOCAL_PROVIDER_CONFIG_ID)
    );
    assert_eq!(
        queued.model_id.as_deref(),
        Some("gemma-4-12B-it-qat-q4_0-gguf")
    );
    let frozen = queued
        .auto_route_identity
        .as_ref()
        .expect("queued Auto-route identity is persisted");
    assert_eq!(frozen.provider_config_id, TEST_LOCAL_PROVIDER_CONFIG_ID);
    assert_eq!(frozen.provider_type, "local_model");
    assert_eq!(frozen.model_id, "gemma-4-12B-it-qat-q4_0-gguf");
    assert_eq!(frozen.provenance, "explicit_session");
    assert_eq!(frozen.route_generation, 1);

    engine
        .open_connection()
        .expect("database writes")
        .execute(
            "UPDATE active_session_configs
             SET local_provider_config_id = 'replacement-config',
                 local_provider_type = 'replacement-provider',
                 model_id = 'replacement-model',
                 local_model_source = 'startup_default',
                 local_route_generation = local_route_generation + 1
             WHERE session_id = ?1",
            params![queued.session_id.as_deref().expect("queued session")],
        )
        .expect("session route mutates after queue acceptance");
    let after_mutation = engine
        .select_queued_messages(queued.session_id.as_deref().expect("queued session"))
        .expect("queued identity reloads")
        .pop()
        .expect("queued row remains");
    assert_eq!(
        after_mutation.auto_route_identity,
        queued.auto_route_identity,
        "queued execution must consume the immutable acceptance identity, not the changed session route"
    );

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn legacy_normal_and_queue_saves_cannot_contaminate_dynamic_route_identity_across_restart() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-legacy-save-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let session = engine
        .ensure_chat_session_with_auto_route_baseline(
            CreateChatSessionRequest {
                agent_id: "agent-legacy-save".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Protected Auto-route".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            verified_test_baseline(crate::gemma::GEMMA_E4B_CANONICAL_ID),
            &installed_model_root(),
        )
        .expect("dynamic session and E4B identity commit");
    for source in ["normal send", "queued send"] {
        let error = engine
            .upsert_session_config(
                &session.id,
                "medium",
                16_384,
                Some("dynamic"),
                Some("dynamic"),
                Some("dynamic"),
            )
            .expect_err(source);
        assert!(matches!(
            error,
            rusqlite::Error::InvalidParameterName(code)
                if code == crate::db::routing_persistence::AUTO_ROUTE_LEGACY_SESSION_CONFIG_FORBIDDEN
        ));
    }
    let queued = engine
        .insert_queued_message(QueueMessageRequest {
            turn_id: Some("turn-protected-queue".to_string()),
            generation_token: Some("generation-protected-queue".to_string()),
            parent_turn_id: None,
            root_turn_id: Some("turn-protected-queue".to_string()),
            turn_kind: Some("root".to_string()),
            agent_id: session.agent_id.clone(),
            message: "Keep this queue on frozen E4B.".to_string(),
            attachments: Vec::new(),
            session_id: Some(session.id.clone()),
            provider_id: Some("dynamic".to_string()),
            model_id: Some("dynamic".to_string()),
            reasoning: Some("medium".to_string()),
            context: Some("16384".to_string()),
            context_budget: Some(16_384),
            steering: None,
            automated_web_grounding_enabled: Some(false),
            dynamic_routing_override: Some(true),
        })
        .expect("queue freezes the protected identity");
    assert_eq!(
        queued
            .auto_route_identity
            .as_ref()
            .map(|value| value.model_id.as_str()),
        Some(crate::gemma::GEMMA_E4B_CANONICAL_ID)
    );
    drop(engine);

    let reopened = PersistenceEngine::initialize_at(path.clone()).expect("database reopens");
    let policy = reopened
        .select_chat_session_route_policy(&session.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        policy.local_provider_id.as_deref(),
        Some(TEST_LOCAL_PROVIDER_CONFIG_ID)
    );
    assert_eq!(policy.local_provider_type.as_deref(), Some("local_model"));
    assert_eq!(
        policy.local_model_id.as_deref(),
        Some(crate::gemma::GEMMA_E4B_CANONICAL_ID)
    );
    assert_eq!(policy.local_source.as_deref(), Some("explicit_session"));
    assert_eq!(policy.route_generation, 1);
    let config = reopened
        .select_session_config(&session.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        config.local_provider_config_id.as_deref(),
        Some(TEST_LOCAL_PROVIDER_CONFIG_ID)
    );
    assert_eq!(config.local_provider_type.as_deref(), Some("local_model"));
    assert_eq!(config.local_route_generation, 1);
    let persisted_queue = reopened.select_queued_messages(&session.id).unwrap();
    assert_eq!(persisted_queue.len(), 1);
    assert_eq!(
        persisted_queue[0]
            .auto_route_identity
            .as_ref()
            .map(|value| value.model_id.as_str()),
        Some(crate::gemma::GEMMA_E4B_CANONICAL_ID)
    );
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

#[test]
fn disabled_dynamic_session_can_save_explicit_e4b_before_reenable() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-disabled-manual-baseline-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let session = engine
        .ensure_chat_session_with_auto_route_baseline(
            CreateChatSessionRequest {
                agent_id: "agent-disabled-manual-baseline".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Disabled manual baseline".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            verified_test_baseline(crate::gemma::GEMMA_E2B_CANONICAL_ID),
            &installed_model_root(),
        )
        .expect("dynamic session starts with the implicit E2B baseline");
    engine
        .open_connection()
        .expect("database opens")
        .execute(
            "UPDATE chat_sessions SET dynamic_routing_override=0 WHERE id=?1",
            params![session.id],
        )
        .expect("Auto-route is disabled without changing the dynamic binding");

    engine
        .upsert_session_config(
            &session.id,
            "medium",
            16_384,
            Some(TEST_LOCAL_PROVIDER_CONFIG_ID),
            Some("local_model"),
            Some(crate::gemma::GEMMA_E4B_CANONICAL_ID),
        )
        .expect("an explicit E4B choice persists while Auto-route is disabled");
    let disabled_policy = engine
        .select_chat_session_route_policy(&session.id)
        .expect("disabled policy reads")
        .expect("disabled policy exists");
    assert_eq!(disabled_policy.session_provider_id, "dynamic");
    assert_eq!(disabled_policy.session_model_id, "dynamic");
    assert_eq!(disabled_policy.dynamic_routing_override, Some(false));
    assert_eq!(
        disabled_policy.local_model_id.as_deref(),
        Some(crate::gemma::GEMMA_E4B_CANONICAL_ID)
    );
    assert_eq!(
        disabled_policy.local_source.as_deref(),
        Some("explicit_session")
    );

    let enabled = engine
        .update_chat_session_dynamic_routing_override(
            &session.id,
            Some(true),
            Some(verified_test_baseline(crate::gemma::GEMMA_E4B_CANONICAL_ID)),
            Some(&installed_model_root()),
        )
        .expect("Auto-route re-enables with the saved explicit E4B baseline");
    assert_eq!(enabled.session.provider_id, "dynamic");
    assert_eq!(enabled.session.model_id, "dynamic");
    assert_eq!(enabled.session.dynamic_routing_override, Some(true));
    assert_eq!(
        enabled
            .receipt
            .model_id
            .as_ref()
            .map(CanonicalModelId::as_str),
        Some(crate::gemma::GEMMA_E4B_CANONICAL_ID)
    );
    let enabled_policy = engine
        .select_chat_session_route_policy(&session.id)
        .expect("enabled policy reads")
        .expect("enabled policy exists");
    assert_eq!(
        enabled_policy.local_model_id.as_deref(),
        Some(crate::gemma::GEMMA_E4B_CANONICAL_ID)
    );

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn auto_route_enable_is_atomic() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-toggle-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-4-12B-it-qat-q4_0-gguf".to_string(),
            title: Some("Toggle Auto-route".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .expect("manual session exists");
    engine
        .upsert_session_config(
            &session.id,
            "medium",
            16_384,
            Some(TEST_LOCAL_PROVIDER_CONFIG_ID),
            Some("local_model"),
            Some("gemma-4-12B-it-qat-q4_0-gguf"),
        )
        .expect("local baseline saves");

    let enabled = engine
        .update_chat_session_dynamic_routing_override(
            &session.id,
            Some(true),
            Some(verified_test_baseline("gemma-4-12B-it-qat-q4_0-gguf")),
            Some(&installed_model_root()),
        )
        .expect("Auto-route enables transactionally");
    assert_eq!(enabled.session.provider_id, "dynamic");
    assert_eq!(enabled.session.model_id, "dynamic");
    let policy = engine
        .select_chat_session_route_policy(&session.id)
        .expect("policy reads")
        .expect("policy exists");
    assert_eq!(
        policy.local_provider_id.as_deref(),
        Some(TEST_LOCAL_PROVIDER_CONFIG_ID)
    );
    assert_eq!(
        policy.local_model_id.as_deref(),
        Some("gemma-4-12B-it-qat-q4_0-gguf")
    );

    let disabled = engine
        .update_chat_session_dynamic_routing_override(
            &session.id,
            Some(false),
            Some(verified_test_baseline("gemma-4-12B-it-qat-q4_0-gguf")),
            Some(&installed_model_root()),
        )
        .expect("manual binding restores");
    assert_eq!(disabled.session.provider_id, TEST_LOCAL_PROVIDER_CONFIG_ID);
    assert_eq!(disabled.session.model_id, "gemma-4-12B-it-qat-q4_0-gguf");

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn dynamic_session_creation_rejects_non_concrete_baseline() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-invalid-baseline-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let result = engine.ensure_chat_session_with_auto_route_baseline(
        CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: None,
            dynamic_routing_override: Some(true),
            workspace_id: None,
        },
        VerifiedAutoRouteBaseline {
            provider_config_id: ProviderConfigurationId::try_from("dynamic".to_string())
                .expect("nonempty invalid config ID"),
            provider_type: ProviderTypeId::try_from("dynamic".to_string())
                .expect("nonempty invalid provider type"),
            model_id: CanonicalModelId::try_from("dynamic".to_string())
                .expect("nonempty invalid model ID"),
            reasoning_depth: "medium".to_string(),
            context_budget: 8_192,
            provenance: AutoRouteProvenance::ExplicitSession,
        },
        &installed_model_root(),
    );
    assert!(result.is_err());
    assert!(engine
        .select_chat_sessions()
        .expect("sessions list")
        .is_empty());

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn dynamic_session_creation_rejects_uninstalled_local_model() {
    let path = std::env::temp_dir().join(format!(
        "oomu-auto-route-uninstalled-baseline-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let result = engine.ensure_chat_session_with_auto_route_baseline(
        CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: None,
            dynamic_routing_override: Some(true),
            workspace_id: None,
        },
        verified_test_baseline("model-that-is-not-installed"),
        &installed_model_root(),
    );
    assert!(result.is_err());
    assert!(engine
        .select_chat_sessions()
        .expect("sessions list")
        .is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn deleting_session_removes_its_encrypted_auto_route_audit() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu-auto-route-audit-delete-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    std::fs::create_dir_all(&temp_dir).expect("temporary directory exists");
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite"))
        .expect("database initializes");
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Private route audit".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .expect("session exists");
    engine
        .insert_dynamic_routing_audit(
            "test-owned prompt",
            "",
            &json!({
                "eventKind": "dynamic_routing_attempt",
                "sessionId": session.id.clone(),
                "turnId": "turn-audit-delete",
            }),
        )
        .expect("audit evidence saves");

    assert!(engine.delete_chat_session_by_id(&session.id).unwrap());
    let ops_connection = engine.open_ops_connection().expect("audit database opens");
    let retained: i64 = ops_connection
        .query_row(
            "SELECT COUNT(*) FROM local_inference_audit WHERE json_extract(metadata_json, '$.sessionId') = ?1",
            params![&session.id],
            |row| row.get(0),
        )
        .expect("audit count reads");
    assert_eq!(retained, 0);

    drop(ops_connection);
    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

fn startup_assignment(model_root: &std::path::Path) -> crate::gemma::StartupModelAssignment {
    crate::gemma::resolve_verified_startup_model_assignment(
        model_root,
        &crate::gemma::StartupModelPreference {
            requested_model_id: crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            selection_source: crate::gemma::StartupModelSelectionSource::CleanDefault,
        },
    )
    .expect("installed E2B startup model resolves")
}

fn dynamic_session(
    engine: &PersistenceEngine,
    agent_id: &str,
    model_id: &str,
) -> ChatSessionRecord {
    engine
        .ensure_chat_session_with_auto_route_baseline(
            CreateChatSessionRequest {
                agent_id: agent_id.to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Reconciled Auto-route".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            {
                let mut baseline = verified_test_baseline(model_id);
                baseline.context_budget = 12_288;
                baseline
            },
            &installed_model_root(),
        )
        .expect("dynamic session exists")
}

#[test]
fn explicit_e4b_session_survives_route_reconciliation() {
    let database_path = std::env::temp_dir().join(format!(
        "oomu-auto-route-explicit-e4b-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(database_path.clone()).expect("database opens");
    let model_root = installed_model_root();
    let session = dynamic_session(
        &engine,
        "agent-explicit-e4b",
        crate::gemma::GEMMA_E4B_CANONICAL_ID,
    );
    let agent_models = HashMap::from([(
        "agent-explicit-e4b".to_string(),
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
    )]);

    let report = engine
        .reconcile_auto_route_session_baselines(
            &model_root,
            &startup_assignment(&model_root),
            &agent_models,
        )
        .expect("reconciliation succeeds");
    let policy = engine
        .select_chat_session_route_policy(&session.id)
        .expect("policy reads")
        .expect("policy exists");

    assert_eq!(report.repaired, 0);
    assert_eq!(report.preserved, 1);
    assert_eq!(
        policy.local_model_id.as_deref(),
        Some(crate::gemma::GEMMA_E4B_CANONICAL_ID)
    );
    assert_eq!(policy.local_source.as_deref(), Some("explicit_session"));

    drop(engine);
    let _ = std::fs::remove_file(database_path);
}

#[test]
fn missing_non_explicit_baseline_uses_verified_startup_assignment() {
    let database_path = std::env::temp_dir().join(format!(
        "oomu-auto-route-startup-repair-{}-{}.db",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(database_path.clone()).expect("database opens");
    let model_root = installed_model_root();
    let session = dynamic_session(
        &engine,
        "agent-without-saved-model",
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
    );
    engine
        .open_connection()
        .expect("database writes")
        .execute(
            "UPDATE active_session_configs
             SET model_id = 'missing-legacy-model', local_model_source = 'legacy_unverified'
             WHERE session_id = ?1",
            params![session.id],
        )
        .expect("legacy fixture saves");

    let report = engine
        .reconcile_auto_route_session_baselines(
            &model_root,
            &startup_assignment(&model_root),
            &HashMap::new(),
        )
        .expect("startup-backed repair succeeds");
    let policy = engine
        .select_chat_session_route_policy(&session.id)
        .expect("policy reads")
        .expect("policy exists");

    assert_eq!(report.repaired, 1);
    assert_eq!(
        policy.local_model_id.as_deref(),
        Some(crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID)
    );
    assert_eq!(policy.local_source.as_deref(), Some("startup_default"));
    let _ = std::fs::remove_file(database_path);
}

#[test]
fn legacy_dynamic_session_repairs_only_unambiguous_baseline() {
    let temp_root = std::env::temp_dir().join(format!(
        "oomu-auto-route-legacy-models-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let unique_root = temp_root.join("unique");
    let ambiguous_root = temp_root.join("ambiguous");
    std::fs::create_dir_all(&unique_root).expect("unique model root exists");
    std::fs::create_dir_all(&ambiguous_root).expect("ambiguous model root exists");
    let installed = installed_model_root();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            installed.join(crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID),
            unique_root.join(crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID),
        )
        .expect("unique E2B link exists");
        for model_id in [
            crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
            crate::gemma::GEMMA_E4B_CANONICAL_ID,
        ] {
            std::os::unix::fs::symlink(installed.join(model_id), ambiguous_root.join(model_id))
                .expect("ambiguous model link exists");
        }
    }
    let database_path = temp_root.join("state.sqlite");
    let engine = PersistenceEngine::initialize_at(database_path).expect("database opens");
    let repaired_session = dynamic_session(
        &engine,
        "agent-legacy-one",
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
    );
    engine
        .open_connection()
        .expect("database reads")
        .execute(
            "UPDATE active_session_configs
             SET model_id = 'models', local_model_source = 'legacy_unverified'
             WHERE session_id = ?1",
            params![repaired_session.id],
        )
        .expect("legacy marker saves");

    let unique_report = engine
        .reconcile_auto_route_session_baselines(
            &unique_root,
            &startup_assignment(&unique_root),
            &HashMap::new(),
        )
        .expect("unique legacy model repairs");
    let repaired = engine
        .select_chat_session_route_policy(&repaired_session.id)
        .expect("repaired policy reads")
        .expect("repaired policy exists");
    assert_eq!(unique_report.repaired, 1);
    assert_eq!(
        repaired.local_model_id.as_deref(),
        Some(crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID)
    );
    assert_eq!(
        repaired.local_source.as_deref(),
        Some("verified_legacy_repair")
    );

    let choice_session = dynamic_session(
        &engine,
        "agent-legacy-many",
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
    );
    engine
        .open_connection()
        .expect("database reads")
        .execute(
            "UPDATE active_session_configs
             SET model_id = 'models', local_model_source = 'legacy_unverified'
             WHERE session_id = ?1",
            params![choice_session.id],
        )
        .expect("ambiguous marker saves");
    let ambiguous_report = engine
        .reconcile_auto_route_session_baselines(
            &ambiguous_root,
            &startup_assignment(&ambiguous_root),
            &HashMap::new(),
        )
        .expect("ambiguous legacy model is handled safely");
    let choice = engine
        .select_chat_session_route_policy(&choice_session.id)
        .expect("choice policy reads")
        .expect("choice policy exists");
    assert_eq!(ambiguous_report.needs_user_choice, 1);
    assert_eq!(choice.local_source.as_deref(), Some("needs_user_choice"));
    assert_eq!(choice.local_model_id.as_deref(), Some("models"));

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_root);
}

#[test]
fn ambiguous_legacy_models_baseline_uses_verified_agent_assignment() {
    let temp_root = std::env::temp_dir().join(format!(
        "oomu-auto-route-agent-models-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    std::fs::create_dir_all(&temp_root).expect("model root exists");
    let installed = installed_model_root();
    #[cfg(unix)]
    for model_id in [
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
        crate::gemma::GEMMA_E4B_CANONICAL_ID,
    ] {
        std::os::unix::fs::symlink(installed.join(model_id), temp_root.join(model_id))
            .expect("model link exists");
    }
    let database_path = temp_root.join("state.sqlite");
    let engine = PersistenceEngine::initialize_at(database_path).expect("database opens");
    let session = dynamic_session(
        &engine,
        "agent-legacy-models",
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
    );
    engine
        .open_connection()
        .expect("database reads")
        .execute(
            "UPDATE active_session_configs
             SET model_id = 'models', local_model_source = 'legacy_unverified'
             WHERE session_id = ?1",
            params![session.id],
        )
        .expect("legacy marker saves");
    let agents = HashMap::from([(
        "agent-legacy-models".to_string(),
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
    )]);

    let report = engine
        .reconcile_auto_route_session_baselines(
            &temp_root,
            &startup_assignment(&temp_root),
            &agents,
        )
        .expect("agent-backed legacy model repairs");
    let policy = engine
        .select_chat_session_route_policy(&session.id)
        .expect("policy reads")
        .expect("policy exists");

    assert_eq!(report.repaired, 1);
    assert_eq!(report.needs_user_choice, 0);
    assert_eq!(
        policy.local_model_id.as_deref(),
        Some(crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID)
    );
    assert_eq!(policy.local_source.as_deref(), Some("agent_assignment"));
    drop(engine);
    let _ = std::fs::remove_dir_all(temp_root);
}
