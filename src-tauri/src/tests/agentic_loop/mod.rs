use super::*;

#[path = "../../test_local_models.rs"]
mod test_local_models;

#[test]
fn missing_delete_preflight_uses_plain_stable_copy() {
    let verifier_reason = "Pre-flight ActionPlan verification failed for 1 issue(s): Shield Gate rejected step 'Delete the specified file': The requested file is not there.";

    assert_eq!(
        preflight_error_code(verifier_reason, "agent_preflight_verification_failed"),
        "delete_target_not_found"
    );
    assert_eq!(
        preflight_halt_message(verifier_reason),
        "That file is not there, so there is nothing to delete. Check the path and try again."
    );
}

#[test]
fn manual_session_identity_signs_completed_tool_receipts_without_keychain_node_state() {
    let identity = SovereignIdentity::initialize_with_session_passphrase(
        "OOMU isolated execution identity 148",
    )
    .expect("manual session identity initializes");
    let response = ExecuteCommandResponse {
        operation: "create_decision_pack".to_string(),
        status: CommandStatus::Completed,
        message: "The four verified decision-pack files were published.".to_string(),
        metrics: None,
        claims: vec![
            "CLAIM decision_pack_file_verified=true kind=workbook sha256=verified".to_string(),
        ],
        verified: true,
        model_used: None,
    };

    let signed = sign_tool_response(response, &identity)
        .expect("a completed task-tool receipt signs from memory-only identity state");

    assert!(signed.verified);
    assert!(signed.claims.iter().any(|claim| {
        claim.contains("operation=create_decision_pack") && claim.contains("node_id=oomu-node-")
    }));
}

pub(crate) fn compile_signed_decision_pack_plan(
    objective: &str,
    output_directory: &str,
) -> Result<(ActionPlan, SovereignIdentity), String> {
    let resolved_paths = contextual_route::ResolvedContextualObjectivePaths {
        objective: objective.to_string(),
        output_directory: output_directory.to_string(),
    };
    let (objective, draft) = plan_coverage::resolve_and_compile_decision_pack(
        objective.to_string(),
        Some(&resolved_paths),
        false,
    )
    .map_err(|error| error.message)?;
    let draft =
        draft.ok_or_else(|| "objective did not compile as a decision-pack contract".to_string())?;
    let draft = plan_coverage::prepare_draft_for_execution(&objective, draft, true)
        .map_err(|error| error.message)?;
    let route = ModelRouteDecision {
        selected_model: ModelMetadata::local_gemma(),
        provider_config_id: None,
        provider_id: Some("local_model".to_string()),
        recommended_model: None,
        requires_principal_authorization: false,
        reason: "The exact grounded native contract compiled deterministically.".to_string(),
        context_excerpt_count: 0,
        context_sources: Vec::new(),
    };
    let identity = SovereignIdentity::initialize_ephemeral();
    let plan = generated_draft_to_plan(
        objective,
        draft,
        route,
        ContextBundle {
            excerpts: Vec::new(),
            claim_sources: Vec::new(),
            inherited_artifact_hashes: Vec::new(),
        },
    );
    sign_plan(plan, &identity)
        .map(|plan| (plan, identity))
        .map_err(|error| error.message)
}

fn diagnostics_draft() -> GeneratedActionPlanDraft {
    GeneratedActionPlanDraft {
        steps: vec![GeneratedPlanStepDraft {
            step: "Collect local system metrics.".to_string(),
            tool: GeneratedToolDraft::SystemDiagnostics {
                principal: "local_principal".to_string(),
            },
            risk_level: GeneratedRiskLevel::Low,
        }],
        exit_condition: "Exit after diagnostics complete.".to_string(),
        generated_text: "{}".to_string(),
        source: crate::gemma::IntentSource::Gemma,
        degraded_reason: None,
    }
}

fn mock_prescriptive_mod_inference(prompt: &str) -> String {
    assert!(prompt.contains(crate::agent_manager::PRESCRIPTIVE_COMPLIANCE_CONTRACT_HEADING));
    [
        "### CLIENT PROFILE STATE",
        "*   State: Jordan is frustrated and confused by duplicate billing.",
        "*   Issues: invoice mismatch, missing cancellation confirmation.",
        "",
        "### RECOMMENDED RESOLUTION PATHS",
        "1.  Check the local billing RAG article for duplicate-charge reversal steps.",
        "2.  Verify account status and open invoice records before responding.",
        "",
        "### EXPERIENCE ENHANCEMENT CHECKS",
        "*   Calibrated Tone: acknowledge friction first, then give one concrete next step.",
        "*   Pitfalls to Avoid: do not promise a refund before system verification.",
    ]
    .join("\n")
}

fn frontend_vertical_layout_parser_mock(output: &str) -> Result<usize, String> {
    let required_headers = [
        "### CLIENT PROFILE STATE",
        "### RECOMMENDED RESOLUTION PATHS",
        "### EXPERIENCE ENHANCEMENT CHECKS",
    ];
    if !output.trim_start().starts_with(required_headers[0]) {
        return Err("vertical payload did not start with the client profile signature".to_string());
    }
    let section_count = required_headers
        .iter()
        .filter(|header| output.contains(**header))
        .count();
    if section_count != required_headers.len() {
        return Err("vertical payload was missing a required operation panel section".to_string());
    }
    Ok(section_count)
}

#[test]
fn agent_worker_panic_payload_becomes_structured_error() {
    let payload = std::panic::catch_unwind(|| panic!("synthetic agent panic")).unwrap_err();
    let error = agent_worker_panic_error(payload);

    assert_eq!(error.code, "agent_worker_panic");
    assert_eq!(error.boundary, "AgenticLoop");
    assert!(error.message.contains("synthetic agent panic"));
}

#[test]
fn agent_session_lease_cleanup_revokes_aborted_execution_authority() {
    let leases = ActuationLeaseManager::default();
    leases
        .grant(
            "actor-test".to_string(),
            "session-aborted",
            vec!["filesystem_write".to_string()],
            vec!["actuation-session:session-aborted".to_string()],
            5 * 60 * 1_000,
            2,
        )
        .expect("session lease is granted");

    {
        let _cleanup = AgentSessionLeaseCleanup::new(
            Some(leases.clone()),
            None,
            "session-aborted".to_string(),
        );
    }

    let status = leases.snapshot();
    assert!(!status.active);
    assert_eq!(status.reason, None);
}

#[test]
fn volatile_persistence_rejects_execution_before_intent_side_effect() {
    let root = std::env::temp_dir().join(format!(
        "oomu-volatile-execution-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let persistence = PersistenceEngine::initialize_volatile_at(root.join("state.sqlite")).unwrap();

    let error =
        require_durable_execution(&persistence, "signed action-plan execution").unwrap_err();
    assert_eq!(error.code, "volatile_persistence_execution_blocked");
    let connection = persistence.open_connection().unwrap();
    let intent_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM intents", [], |row| row.get(0))
        .unwrap();
    let action_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM actions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(intent_count, 0);
    assert_eq!(action_count, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn deleted_session_revokes_background_execution_before_effect_and_terminal_persistence() {
    let root = std::env::temp_dir().join(format!(
        "oomu-agent-execution-origin-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let session = persistence
        .ensure_chat_session_with_id(
            "session-agent-execution",
            crate::db::CreateChatSessionRequest {
                agent_id: "agent-execution".to_string(),
                provider_id: "provider-execution".to_string(),
                model_id: "model-execution".to_string(),
                title: Some("Background execution ownership".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            },
        )
        .unwrap();
    let turn_context = AgentPlanExecutionTurnContext {
        turn_id: "turn-agent-execution".to_string(),
        generation_token: "generation-agent-execution".to_string(),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        project_id: None,
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        parent_turn_id: None,
        root_turn_id: "turn-agent-execution".to_string(),
        turn_kind: "root".to_string(),
        reasoning: Some("medium".to_string()),
        context_budget: Some(8_192),
        primary_route_id: Some("provider-execution:model-execution".to_string()),
        fallback_route_id: None,
        dynamic_routing_enabled: false,
        automated_web_grounding_enabled: false,
        attachment_grants: vec![AgentPlanAttachmentGrant {
            name: "request.txt".to_string(),
            mime_type: "text/plain".to_string(),
            byte_count: 12,
        }],
        created_at_ms: 1,
    };
    let persistence_context = turn_context.persistence_context().unwrap();
    persistence
        .ensure_chat_turn_for_native_action(&persistence_context)
        .unwrap();
    let context_json = serde_json::to_string(&turn_context).unwrap();
    persistence
        .begin_agent_execution(
            "execution-delete-guard",
            "plan-delete-guard",
            &persistence_context,
            &context_json,
        )
        .unwrap();
    let guard = AgentExecutionOriginGuard {
        execution_id: "execution-delete-guard".to_string(),
        plan_id: "plan-delete-guard".to_string(),
        context: persistence_context,
        context_json: context_json.clone(),
        persistence: persistence.clone(),
        stream_start_after_log_id: 0,
    };
    guard.ensure_current().unwrap();

    assert!(persistence.delete_chat_session_by_id(&session.id).unwrap());
    assert_eq!(
        guard.ensure_current().unwrap_err().code,
        "agent_execution_origin_stale"
    );
    assert_eq!(
        guard
            .finalize(
                "completed",
                Some("must not persist"),
                "info",
                "completed",
                "must not persist",
                None,
            )
            .unwrap_err()
            .code,
        "agent_execution_origin_stale"
    );

    let connection = persistence.open_connection().unwrap();
    let status: String = connection
        .query_row(
            "SELECT status FROM agent_executions WHERE execution_id = ?1",
            rusqlite::params!["execution-delete-guard"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "cancelled");
    let persisted_context: String = connection
        .query_row(
            "SELECT context_json FROM agent_executions WHERE execution_id = ?1",
            rusqlite::params!["execution-delete-guard"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted_context, context_json);
    let forbidden_receipts: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM chat_messages WHERE content = 'must not persist'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(forbidden_receipts, 0);

    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn verify_prescriptive_mod_compilation() {
    let event = BackgroundHookObjective {
        mod_id: "ai.eldris.mods.customer-support".to_string(),
        source_path: "/tmp/customer_transcript.json".to_string(),
        raw_content:
            r#"{"customer":"Jordan","sentiment":"frustrated","issue":"duplicate billing"}"#
                .to_string(),
        detected_at_ms: 1_795_636_800_000,
    };

    let prompt = background_hook_prompt(&event);

    assert!(prompt.contains("Background event-driven OOMU mod hook fired."));
    assert!(prompt.contains(crate::agent_manager::PRESCRIPTIVE_COMPLIANCE_CONTRACT_HEADING));
    assert!(prompt.contains("### CLIENT PROFILE STATE"));
    assert!(prompt.contains("### RECOMMENDED RESOLUTION PATHS"));
    assert!(prompt.contains("### EXPERIENCE ENHANCEMENT CHECKS"));
    assert!(prompt
        .trim_end()
        .ends_with("*   Pitfalls to Avoid: [High-risk friction points to actively block]"));

    let generated = mock_prescriptive_mod_inference(&prompt);
    assert!(generated.contains("### CLIENT PROFILE STATE"));
    assert!(generated.contains("### RECOMMENDED RESOLUTION PATHS"));
    assert!(generated.contains("### EXPERIENCE ENHANCEMENT CHECKS"));
    assert_eq!(
        frontend_vertical_layout_parser_mock(&generated).expect("layout parser routes payload"),
        3
    );
}

#[test]
fn explicit_web_search_with_active_consent_normalizes_diagnostics_draft_to_search() {
    let draft = normalize_web_search_plan_draft(
        "Search the web: is the World Cup happening right now?",
        diagnostics_draft(),
        true,
    );

    assert_eq!(draft.steps.len(), 1);
    match &draft.steps[0].tool {
        GeneratedToolDraft::SovereignDuckDuckGoSearch { query, max_results } => {
            assert!(query.contains("World Cup"));
            assert_eq!(*max_results, Some(5));
        }
        tool => panic!("expected sovereign search draft, got {tool:?}"),
    }
    assert!(draft
        .degraded_reason
        .as_deref()
        .unwrap_or("")
        .contains("normalized"));
}

#[test]
fn web_search_normalization_preserves_local_file_requests() {
    let draft = normalize_web_search_plan_draft(
        "Search /Users/example/project/src/main.rs for TODO comments.",
        diagnostics_draft(),
        true,
    );

    assert!(matches!(
        draft.steps[0].tool,
        GeneratedToolDraft::SystemDiagnostics { .. }
    ));
}

#[test]
fn web_search_normalization_preserves_local_telemetry_archive_requests() {
    let draft = normalize_web_search_plan_draft(
            "OOMU, perform a system-level operational audit of our workspace. Run an AppleScript query to determine if VS Code, Terminal, or standard editor processes are currently active on macOS. Scan ~/.oomu/mods and package the result into telemetry_audit.tar.gz.",
            diagnostics_draft(),
            true,
        );

    assert!(matches!(
        draft.steps[0].tool,
        GeneratedToolDraft::SystemDiagnostics { .. }
    ));
}

#[test]
fn web_search_normalization_keeps_existing_search_step() {
    let mut draft = diagnostics_draft();
    draft.steps = vec![GeneratedPlanStepDraft {
        step: "Search current sources.".to_string(),
        tool: GeneratedToolDraft::SovereignDuckDuckGoSearch {
            query: "Red Sox score today".to_string(),
            max_results: Some(3),
        },
        risk_level: GeneratedRiskLevel::Low,
    }];

    let normalized =
        normalize_web_search_plan_draft("Search online for the Red Sox score today.", draft, true);

    match &normalized.steps[0].tool {
        GeneratedToolDraft::SovereignDuckDuckGoSearch { query, max_results } => {
            assert_eq!(query, "Red Sox score today");
            assert_eq!(*max_results, Some(3));
        }
        tool => panic!("expected existing search draft, got {tool:?}"),
    }
}

#[test]
fn ambient_search_authorizes_freshness_but_not_loose_lookup_words() {
    for objective in [
        "What is the Red Sox score today?",
        "Check the latest Red Sox score.",
    ] {
        let normalized = normalize_web_search_plan_draft(objective, diagnostics_draft(), true);
        assert!(matches!(
            normalized.steps[0].tool,
            GeneratedToolDraft::SovereignDuckDuckGoSearch { .. }
        ));
    }

    let loose_lookup = normalize_web_search_plan_draft(
        "Look up the Red Sox farm system.",
        diagnostics_draft(),
        true,
    );
    assert!(matches!(
        loose_lookup.steps[0].tool,
        GeneratedToolDraft::SystemDiagnostics { .. }
    ));

    let normalized = normalize_web_search_plan_draft(
        "Search the web for the Red Sox score today.",
        diagnostics_draft(),
        false,
    );
    assert!(matches!(
        normalized.steps[0].tool,
        GeneratedToolDraft::SovereignDuckDuckGoSearch { .. }
    ));

    let freshness_without_ambient = normalize_web_search_plan_draft(
        "What is the Red Sox score today?",
        diagnostics_draft(),
        false,
    );
    assert!(matches!(
        freshness_without_ambient.steps[0].tool,
        GeneratedToolDraft::SystemDiagnostics { .. }
    ));
}

#[test]
fn private_calendar_objective_rejects_a_model_generated_search_plan() {
    let mut draft = diagnostics_draft();
    draft.steps[0].tool = GeneratedToolDraft::SovereignDuckDuckGoSearch {
        query: "my calendar today".to_string(),
        max_results: Some(5),
    };

    let error = validate_planner_draft_for_execution(
        "Check my calendar and let me know what I have going on today",
        &draft,
        true,
    )
    .expect_err("private calendar data must not fall back to search");

    assert_eq!(error.code, "private_app_web_fallback_blocked");
}

fn temporary_agent_manager(test_name: &str) -> AgentManager {
    let mut db_path = std::env::temp_dir();
    db_path.push(format!(
        "oomu-agentic-loop-{test_name}-{}.db",
        unix_time_ms()
    ));
    AgentManager::initialize_at(db_path).expect("agent manager initializes")
}

fn planner_provider_config(
    id: &str,
    provider_id: &str,
    model_id: &str,
) -> crate::agent_manager::ConfiguredProvider {
    crate::agent_manager::ConfiguredProvider {
        id: id.to_string(),
        provider_id: provider_id.to_string(),
        provider_name: provider_id.to_string(),
        auth_method: "api_key".to_string(),
        base_url: String::new(),
        api_key_label: String::new(),
        api_key: Some("test-key".to_string()),
        credential_configured: true,
        custom_model_ids: model_id.to_string(),
        auto_route_target: false,
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}

#[test]
fn systems_programming_objective_escalates_planner_to_auto_route_cloud_target() {
    let manager = temporary_agent_manager("cloud-planner-route");
    let mut provider = planner_provider_config("prov-planner", "google", "gemini-3.5-flash");
    provider.provider_name = "Google Gemini".to_string();
    provider.auto_route_target = true;
    manager
        .upsert_provider_config(provider)
        .expect("auto route target saves");

    let target = resolve_planning_execution_target(
        Some(&manager),
        "Design an unsafe Rust memory allocator with lock-free concurrency.",
        ModelRoutePreference::LocalGemma,
        None,
        None,
    )
    .expect("planner route resolves");

    match &target {
        PlannerExecutionTarget::Cloud(target) => {
            assert_eq!(target.provider_id, "google");
            assert_eq!(target.model_id, "gemini-3.5-flash");
            assert_eq!(
                target.reason,
                "Using Google Gemini/gemini-3.5-flash for this request."
            );
        }
        PlannerExecutionTarget::Local { reason, .. } => {
            panic!("expected cloud planner target, got local: {reason}")
        }
    }

    let route = ModelRouter::route(
        "Design an unsafe Rust memory allocator with lock-free concurrency.",
        ModelRoutePreference::LocalGemma,
        &diagnostics_draft(),
        0,
        &target,
    );
    assert_eq!(route.selected_model.locality, "remote");
    assert_eq!(route.selected_model.provider, "Google Gemini");
    assert_eq!(
        route.reason,
        "Using Google Gemini/gemini-3.5-flash for this request."
    );
}

#[test]
fn selected_cloud_session_route_unifies_planner_target() {
    let manager = temporary_agent_manager("selected-cloud-planner-route");
    let mut provider = planner_provider_config("prov-planner", "google", "gemini-3.5-flash");
    provider.provider_name = "Google Gemini".to_string();
    manager
        .upsert_provider_config(provider)
        .expect("provider config saves");

    let target = resolve_planning_execution_target(
        Some(&manager),
        "Draft a launch checklist.",
        ModelRoutePreference::GeminiPro,
        Some("prov-planner"),
        Some("gemini-3.5-flash"),
    )
    .expect("planner route resolves");

    match &target {
        PlannerExecutionTarget::Cloud(target) => {
            assert_eq!(target.provider_id, "google");
            assert_eq!(target.model_id, "gemini-3.5-flash");
            assert_eq!(
                target.reason,
                "Using Google Gemini/gemini-3.5-flash for this request."
            );
        }
        PlannerExecutionTarget::Local { reason, .. } => {
            panic!("expected selected cloud planner target, got local: {reason}")
        }
    }
}

#[test]
fn plan_route_reason_is_one_plain_sentence_without_internal_context_ids() {
    let model = ModelMetadata {
        name: "gemini-3.5-flash".to_string(),
        version: "API bridge".to_string(),
        provider: "Google Gemini".to_string(),
        locality: "remote".to_string(),
    };
    let reason = plain_plan_route_reason(&model, 5, true);

    assert_eq!(
        reason,
        "Using Google Gemini/gemini-3.5-flash for this request with 5 notes from this project and your recent chat."
    );
    for banned in [
        "cognition",
        "hydrated",
        "signals",
        "ffi",
        "artifact excerpt",
        "Agent-first",
        "imported_",
    ] {
        assert!(
            !reason.contains(banned),
            "unexpected user-facing token: {banned}"
        );
    }
    assert!(reason.ends_with('.'));
    assert!(!reason.trim_end_matches('.').contains(". "));
}

#[test]
fn configured_local_session_route_preserves_the_selected_on_device_model() {
    let manager = temporary_agent_manager("selected-local-planner-route");
    let mut provider =
        planner_provider_config("prov-local", "local_model", "gemma-4-E4B-it-qat-q4_0-gguf");
    provider.auth_method = "custom".to_string();
    provider.api_key = None;
    provider.credential_configured = false;
    manager
        .upsert_provider_config(provider)
        .expect("local provider config saves");

    let target = resolve_planning_execution_target(
        Some(&manager),
        "Prepare a release recovery meeting.",
        ModelRoutePreference::LocalGemma,
        Some("prov-local"),
        Some("gemma-4-E4B-it-qat-q4_0-gguf"),
    )
    .expect("configured local route resolves");

    match target {
        PlannerExecutionTarget::Local { model_id, reason } => {
            assert_eq!(model_id.as_deref(), Some("gemma-4-E4B-it-qat-q4_0-gguf"));
            assert!(reason.contains("on-device"));
        }
        PlannerExecutionTarget::Cloud(target) => {
            panic!("configured local route must not become cloud: {target:?}")
        }
    }
}

#[test]
fn dynamic_planner_uses_the_accepted_sessions_local_baseline_not_the_agent_default() {
    let base = std::env::temp_dir().join(format!(
        "oomu-planner-session-baseline-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let persistence = PersistenceEngine::initialize_at(base.join("state.sqlite")).unwrap();
    let session = persistence
        .ensure_chat_session_with_auto_route_baseline(
            crate::db::CreateChatSessionRequest {
                agent_id: "agent-session-baseline".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Scenario 4 route".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            crate::db::VerifiedAutoRouteBaseline {
                provider_config_id: crate::db::ProviderConfigurationId::try_from(
                    "prov-local-agentic-test".to_string(),
                )
                .expect("provider configuration ID"),
                provider_type: crate::db::ProviderTypeId::try_from("local_model".to_string())
                    .expect("provider type"),
                model_id: crate::db::CanonicalModelId::try_from(
                    "gemma-4-E4B-it-qat-q4_0-gguf".to_string(),
                )
                .expect("model ID"),
                reasoning_depth: "medium".to_string(),
                context_budget: 8_192,
                provenance: crate::db::AutoRouteProvenance::ExplicitSession,
            },
            &test_local_models::root(),
        )
        .unwrap();
    let agent = AgentConfig {
        id: "agent-session-baseline".to_string(),
        name: "OOMU".to_string(),
        system_prompt: "Plan safely.".to_string(),
        model_id: "stale-agent-model".to_string(),
        provider_id: "stale-agent-provider".to_string(),
        description: "Test agent".to_string(),
        image: None,
        personality_profile: serde_json::json!({}).to_string(),
        favorited: false,
        status: crate::agent_manager::AgentConfigStatus::Active,
        created_at_ms: 1,
        updated_at_ms: 1,
    };

    let baseline = planner_baseline_fields(&persistence, Some(&session.id), &agent)
        .expect("the accepted session baseline is authoritative");

    assert_eq!(baseline.0, "prov-local-agentic-test");
    assert_eq!(baseline.1, "gemma-4-E4B-it-qat-q4_0-gguf");
    let policy = persistence
        .select_chat_session_route_policy(&session.id)
        .expect("the typed session baseline reads")
        .expect("the typed session baseline exists");
    assert_eq!(policy.local_provider_type.as_deref(), Some("local_model"));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn ordinary_objective_remains_on_local_planner() {
    let target = resolve_planning_execution_target(
        None,
        "List the files in this project.",
        ModelRoutePreference::LocalGemma,
        None,
        None,
    )
    .expect("planner route resolves");

    match target {
        PlannerExecutionTarget::Local { reason, .. } => {
            assert!(reason.contains("Local Gemma selected"));
        }
        PlannerExecutionTarget::Cloud(target) => {
            panic!("ordinary objective should not use cloud target: {target:?}")
        }
    }
}

#[test]
fn system_gemini_auto_route_fallback_reaches_cloud_target_before_credential_resolution() {
    let target = resolve_planning_execution_target(
        None,
        "Reconcile the decision pack and research official freight conditions.",
        ModelRoutePreference::LocalGemma,
        Some("gemini"),
        Some(crate::settings::DYNAMIC_CLOUD_FALLBACK_MODEL_ID),
    )
    .expect("system Gemini planner target resolves");

    match target {
        PlannerExecutionTarget::Cloud(target) => {
            assert_eq!(target.provider_id, "gemini");
            assert!(target.provider_config_id.is_none());
            assert_eq!(target.provider_name, "Google Gemini");
            assert_eq!(
                target.model_id,
                crate::settings::DYNAMIC_CLOUD_FALLBACK_MODEL_ID
            );
            assert!(target.api_key.is_none());
        }
        PlannerExecutionTarget::Local { reason, .. } => {
            panic!("expected system Gemini cloud target, got local: {reason}")
        }
    }
}

fn completed_response() -> ExecuteCommandResponse {
    ExecuteCommandResponse {
        operation: "file_read".to_string(),
        status: crate::shield_gate::CommandStatus::Completed,
        message: "Read verified workspace content.".to_string(),
        metrics: None,
        claims: vec!["CLAIM file_exists path=workspace/readme.md min_bytes=12".to_string()],
        verified: true,
        model_used: Some(ModelMetadata::local_gemma()),
    }
}

fn sensor_test_plan() -> ActionPlan {
    ActionPlan {
        id: "plan-sensor-test".to_string(),
        objective: "Fix the backend compile error.".to_string(),
        intent: StructuredIntent {
            objective: "Fix the backend compile error.".to_string(),
            category: IntentCategory::ProjectAnalysis,
            source: crate::gemma::IntentSource::Gemma,
            degraded_reason: None,
        },
        steps: vec![Step {
            step: "Compile the backend.".to_string(),
            tool: Tool::CodebaseCompile {
                target: "backend".to_string(),
            },
            risk_level: RiskLevel::High,
        }],
        exit_condition: "Exit after the backend compile passes.".to_string(),
        logical_certificate: LogicalCertificate::unsigned(Vec::new(), Vec::new(), String::new()),
        trusted_automatic_execution: true,
        model_route: ModelRouteDecision {
            selected_model: ModelMetadata::local_gemma(),
            provider_config_id: None,
            provider_id: Some("local_model".to_string()),
            recommended_model: None,
            requires_principal_authorization: false,
            reason: "test route".to_string(),
            context_excerpt_count: 0,
            context_sources: Vec::new(),
        },
        parent_artifact_hashes: Vec::new(),
    }
}

fn approved_execution_turn_context() -> AgentPlanExecutionTurnContext {
    AgentPlanExecutionTurnContext {
        turn_id: "turn-approved-file".to_string(),
        generation_token: "generation-approved-file".to_string(),
        session_id: "session-approved-file".to_string(),
        agent_id: "agent-oomu".to_string(),
        project_id: None,
        provider_id: "local".to_string(),
        model_id: "gemma".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-approved-file".to_string(),
        turn_kind: "root".to_string(),
        reasoning: None,
        context_budget: None,
        primary_route_id: None,
        fallback_route_id: None,
        dynamic_routing_enabled: false,
        automated_web_grounding_enabled: false,
        attachment_grants: Vec::new(),
        created_at_ms: 1,
    }
}

#[test]
fn approved_registered_pdf_creation_provisions_one_native_actuation_step() {
    let _ = crate::artifacts::register_file_task_tool();
    let identity = SovereignIdentity::initialize_ephemeral();
    let mut plan = sensor_test_plan();
    plan.id = "plan-create-approved-pdf".to_string();
    plan.objective = "Create a test PDF in Downloads.".to_string();
    plan.intent.objective = plan.objective.clone();
    let destination = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .expect("test home is available")
        .join("Downloads")
        .join("verification-canary.pdf")
        .to_string_lossy()
        .to_string();
    plan.steps = vec![Step {
        step: "Create the approved PDF.".to_string(),
        tool: Tool::RegisteredTaskTool(
            crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(
                "create_file",
                serde_json::json!({
                    "file": {
                        "title": "Verification canary",
                        "content": "Hello World",
                        "locale": "en-US",
                        "format": "pdf",
                        "destinationPath": destination
                    }
                }),
            ),
        ),
        risk_level: RiskLevel::High,
    }];
    plan.exit_condition = "Exit after the approved PDF exists and is verified.".to_string();
    let plan = sign_plan(plan, &identity).expect("test plan is signed");
    let request = AgentPlanExecutionRequest {
        plan,
        turn_context: approved_execution_turn_context(),
        principal_approved: true,
        authority_proof_id: None,
    };

    assert_eq!(
        approved_agent_plan_actuation_budget(&request, &identity, 0)
            .expect("approved create_file plan has a native actuation budget")
            .max_steps,
        1
    );
    assert_eq!(
        approved_agent_plan_actuation_budget(&request, &identity, 1)
            .expect("a completed checkpoint requires no new actuation authority")
            .max_steps,
        0
    );

    let mut unapproved = request;
    unapproved.principal_approved = false;
    let error = approved_agent_plan_actuation_budget(&unapproved, &identity, 0)
        .expect_err("the runtime must reject execution without explicit approval");
    assert_eq!(error.code, "principal_approval_required");
}

#[test]
fn signed_trusted_read_only_plan_needs_no_separate_principal_approval() {
    let identity = SovereignIdentity::initialize_ephemeral();
    let mut plan = sensor_test_plan();
    plan.id = "plan-read-only-status".to_string();
    plan.objective = "Inspect the current project without changing it.".to_string();
    plan.intent.objective = plan.objective.clone();
    plan.steps = vec![Step {
        step: "Check the working tree.".to_string(),
        tool: Tool::TerminalExecute {
            executable: "/usr/bin/git".to_string(),
            args: vec!["status".to_string(), "--short".to_string()],
            env: std::collections::BTreeMap::new(),
            cwd: Some(
                crate::shield_gate::development_repo_root()
                    .to_string_lossy()
                    .into_owned(),
            ),
            timeout: Some(crate::tools::terminal_contract::DEFAULT_TERMINAL_TIMEOUT_MS),
        },
        risk_level: RiskLevel::Low,
    }];
    plan.trusted_automatic_execution = true;
    let request = AgentPlanExecutionRequest {
        plan: sign_plan(plan, &identity).expect("trusted read-only plan is signed"),
        turn_context: approved_execution_turn_context(),
        principal_approved: false,
        authority_proof_id: None,
    };

    let budget = approved_agent_plan_actuation_budget(&request, &identity, 0)
        .expect("signed trusted read-only work uses the user's originating request");
    assert_eq!(budget.max_steps, 0);
    assert!(budget.operation_classes.is_empty());
}

#[test]
fn action_plan_execution_rechecks_immutable_search_authority() {
    let mut plan = sensor_test_plan();
    plan.objective = "Search online for Blackpink tour dates.".to_string();
    plan.intent.objective = plan.objective.clone();
    plan.steps[0].tool = Tool::SovereignDuckDuckGoSearch {
        query: "Blackpink tour dates".to_string(),
        max_results: Some(5),
    };

    validate_action_plan_web_search_authority(&plan, true)
        .expect("explicit originating request is authorized with ambient Search on");
    validate_action_plan_web_search_authority(&plan, false)
        .expect("explicit originating request is authorized with ambient Search off");

    plan.objective = "Check the latest Blackpink tour schedule.".to_string();
    plan.intent.objective = plan.objective.clone();
    validate_action_plan_web_search_authority(&plan, true)
        .expect("ambient Search authorizes a bounded freshness request");
    let implicit_disabled = validate_action_plan_web_search_authority(&plan, false)
        .expect_err("freshness alone cannot authorize search while ambient Search is off");
    assert_eq!(implicit_disabled.code, "web_search_not_authorized");

    plan.steps[0].tool = Tool::RegisteredTaskTool(
        crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(
            crate::tools::evidence_artifacts::COMPARISON_OPERATION,
            serde_json::json!({
                "outputPath": "ship_test_04/background_agent_comparison.md",
                "locale": "en-US"
            }),
        ),
    );
    plan.objective = "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison to ship_test_04/background_agent_comparison.md in my testing folder. Include URLs, access times, explicit limitations, and a section explaining what this implies for OOMU. Do not claim completion until the file exists and you have read it back.".to_string();
    plan.intent.objective = plan.objective.clone();
    validate_action_plan_web_search_authority(&plan, true).expect(
        "explicit primary-or-official source research is authorized with ambient Search on",
    );
    validate_action_plan_web_search_authority(&plan, false).expect(
        "explicit primary-or-official source research is authorized with ambient Search off",
    );

    plan.objective = "Search the web for what is on my calendar today.".to_string();
    plan.intent.objective = plan.objective.clone();
    let private = validate_action_plan_web_search_authority(&plan, true)
        .expect_err("private app data cannot be substituted with search");
    assert_eq!(private.code, "private_app_web_fallback_blocked");
}

#[tokio::test]
async fn approved_plan_verification_precedes_permission_preflight() {
    let root = std::env::temp_dir().join(format!(
        "oomu-plan-permission-order-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let mut plan = sensor_test_plan();
    plan.id = format!("plan-permission-order-{}", unix_time_ms());
    plan.steps[0] = Step {
        step: "Write an external file.".to_string(),
        tool: Tool::FileWrite {
            path: root.join("external.txt").display().to_string(),
            content: "never written".to_string(),
        },
        risk_level: RiskLevel::High,
    };
    // The unsigned certificate is deliberately invalid. With no AppHandle,
    // permission preflight would report `permission_prompt_unavailable` if it
    // ran before the cryptographic and Logical Certificate checks.
    let failure_mlc = crate::settings::app_data_root()
        .join("logs")
        .join("mlc")
        .join(format!("{}-failure.md", plan.id));

    let error = execute_action_plan_inner(
        plan,
        persistence,
        None,
        SovereignIdentity::initialize_ephemeral(),
        GemmaService::new_loading(),
        None,
        None,
        Some("permission-order-session".to_string()),
        Vec::new(),
        None,
        None,
        true,
        false,
        None,
        None,
    )
    .await
    .expect_err("an unsigned approved plan must fail before requesting permission");

    assert_eq!(error.code, "preflight_verification_failed");
    assert_eq!(error.boundary, "MlcVerifier");
    assert!(!root.join("external.txt").exists());
    if let Some(path) = error.mlc_path {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_file(failure_mlc);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_compile_response_generates_sensor_payload_and_directive() {
    let plan = sensor_test_plan();
    let response = ExecuteCommandResponse {
            operation: "codebase_compile".to_string(),
            status: crate::shield_gate::CommandStatus::Failed,
            message: "backend failed while running cargo check. Exit code: Some(101).\n\n[check] cargo check exit=Some(101) timed_out=false\nstdout:\nchecking oomu\nstderr:\nerror[E0425]: cannot find value `missing` in this scope\n --> src/lib.rs:12:9"
                .to_string(),
            metrics: None,
            claims: vec!["CLAIM codebase_compile target=backend success=false phases=1".to_string()],
            verified: true,
            model_used: Some(ModelMetadata::local_gemma()),
        };

    let payload = sensor_payload_from_failed_output(&plan, 0, &response)
        .expect("failed compile produces sensor payload");
    assert_eq!(payload.step_id, "plan-sensor-test:step-1");
    assert_eq!(payload.tool_executed, "codebase_compile");
    assert_eq!(payload.exit_code, 101);
    assert!(payload.stdout.contains("checking oomu"));
    assert!(payload.stderr.contains("cannot find value"));

    let directive = generate_self_healing_directive(&payload);
    assert!(directive.contains("[OOMU COMPILER UPDATE: SYSTEM RESOLUTION REQUIRED]"));
    assert!(directive.contains("Instruction to the expert panel"));
    assert!(directive.contains("Re-run compilation to verify the fix."));
}

#[tokio::test]
async fn failed_command_sensor_update_appends_to_memory_ledger() {
    let root = std::env::temp_dir().join(format!(
        "oomu-agentic-sensor-ledger-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp sensor ledger root is created");
    let ledger = MemoryLedger::initialize_at(root.join("oomu_ops.sqlite"))
        .expect("sensor ledger initializes");
    let payload = SensorUpdatePayload {
        step_id: "plan-sensor-test:step-1".to_string(),
        tool_executed: "shell_command".to_string(),
        exit_code: 1,
        stdout: "pytest collected 1 item".to_string(),
        stderr: "SyntaxError: invalid syntax".to_string(),
    };
    let directive = generate_self_healing_directive(&payload);

    commit_sensor_update_to_ledger(
        Some(ledger.clone()),
        "session-sensor-test".to_string(),
        payload.clone(),
        directive.clone(),
    )
    .await
    .expect("sensor update commits to ledger");

    let rows = ledger
        .select_runtime_sensor_updates_for_mission_sync("session-sensor-test")
        .expect("sensor update rows load");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tool_executed, "shell_command");
    assert_eq!(rows[0].exit_code, 1);
    assert!(rows[0].stderr.contains("SyntaxError"));
    assert!(rows[0].directive.contains("SYSTEM RESOLUTION REQUIRED"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn self_healing_diagnostic_revokes_automatic_execution_at_limit() {
    let payload = SensorUpdatePayload {
        step_id: "plan-sensor-test:step-1".to_string(),
        tool_executed: "codebase_compile".to_string(),
        exit_code: 101,
        stdout: String::new(),
        stderr: "error: expected item".to_string(),
    };
    let directive = generate_self_healing_directive(&payload);
    let diagnostic = self_healing_diagnostic_report(
        "Fix the compile error.",
        &payload,
        &directive,
        SELF_HEALING_MAX_ATTEMPTS,
        SELF_HEALING_MAX_ATTEMPTS,
    );

    assert!(diagnostic.contains("Self-healing paused after 3 of 3 attempts."));
    assert!(diagnostic.contains("Automatic execution privileges are revoked"));
    assert!(diagnostic.contains("error: expected item"));
}

#[test]
fn completed_action_requires_verified_content_and_claims() {
    let mut response = completed_response();
    validate_completed_action_output(&response).expect("valid response");

    response.verified = false;
    assert!(validate_completed_action_output(&response).is_err());

    response.verified = true;
    response.claims = vec!["CLAIM tool_error operation=file_read".to_string()];
    assert!(validate_completed_action_output(&response).is_err());
}

#[test]
fn degraded_and_unsupported_plans_fail_before_execution() {
    let mut degraded = diagnostics_draft();
    degraded.source = IntentSource::Degraded;
    degraded.degraded_reason = Some("provider returned no action-plan JSON".to_string());
    let error = validate_planner_draft_for_execution("Run diagnostics.", &degraded, false)
        .expect_err("degraded plan must not execute");
    assert_eq!(error.code, "planner_output_unusable");
    assert!(!error.message.contains("provider returned"));
    assert!(!error.message.contains("characters"));
    assert!(error.message.contains("No action was executed"));

    let mut unsupported = diagnostics_draft();
    unsupported.steps[0].tool = GeneratedToolDraft::Unsupported {
        requested: "missing destination path".to_string(),
    };
    let error = validate_planner_draft_for_execution("Run diagnostics.", &unsupported, false)
        .expect_err("unsupported clarification must fail closed");
    assert_eq!(error.code, "planner_clarification_required");
    assert!(error.message.contains("No action was executed"));
}

#[test]
fn production_schema_invalid_plan_is_halted_before_execution() {
    let draft = crate::gemma::generated_plan_from_text(
            "Inspect the local workspace.".to_string(),
            r#"{"steps":[{"step":"Inspect workspace","tool":{"kind":"file_list","path":"."}}],"exit_condition":"Stop after inspection."}"#.to_string(),
        );

    assert!(matches!(draft.source, IntentSource::Degraded));
    let error = validate_planner_draft_for_execution("Inspect the local workspace.", &draft, false)
        .expect_err("schema-invalid production output must halt before execution");
    assert_eq!(error.code, "planner_output_unusable");
    assert!(error.message.contains("No action was executed"));
}

#[test]
fn visual_file_write_requires_explicit_path_and_content() {
    let action = WorkflowAction {
        id: "write-1".to_string(),
        kind: WorkflowActionKind::FileWrite,
        label: "Write requested document".to_string(),
        path: None,
        content: None,
        scope: None,
        dependencies: Vec::new(),
    };
    let error = workflow_action_to_step(0, action)
        .expect_err("missing visual workflow inputs must be rejected");
    assert_eq!(error.code, "workflow_action_input_missing");
    assert!(error.message.contains("no default value was substituted"));
}

#[test]
fn signer_rejects_empty_or_error_outputs() {
    let mut response = completed_response();
    response.message.clear();
    assert!(validate_signable_tool_response(&response).is_err());

    response.message = "Read failed.".to_string();
    response.claims = vec!["CLAIM tool_error operation=file_read".to_string()];
    assert!(validate_signable_tool_response(&response).is_err());
}

#[test]
fn semantic_evidence_is_hash_bound_and_detailed() {
    let reasoning = "semantic_pass=true operation=document_index score=0.8400; factors=coverage:0.74,path:0.10; decision=retain verified local document";
    let claim = semantic_evidence_claim(0.84, reasoning);
    let evidence = semantic_evidence_from_claims(&[claim.clone()])
        .expect("semantic evidence parses")
        .expect("semantic evidence exists");
    assert_eq!(evidence.relevance_score, "0.8400");

    let tampered = claim.replace("reasoning_hash=", "reasoning_hash=bad");
    assert!(semantic_evidence_from_claims(&[tampered]).is_err());
}

#[test]
fn semantic_evidence_requires_verifier_reasoning_markers_before_signing() {
    let claim = semantic_evidence_claim(
        0.84,
        "A detailed but unstructured explanation is not enough for final MLC verification.",
    );

    assert!(semantic_evidence_from_claims(&[claim]).is_err());
}

#[test]
fn duckduckgo_search_receipt_uses_observed_response_fields_without_fabricated_score() {
    let response = crate::sovereign_search::SovereignSearchResponse {
        query: "latest Blackpink tour schedule".to_string(),
        engine: "duckduckgo_lite_static".to_string(),
        result_count: 1,
        results: vec![crate::sovereign_search::SovereignSearchResult {
            title: "BLACKPINK Tour".to_string(),
            url: "https://example.com/blackpink-tour".to_string(),
            snippet: "Current tour schedule details.".to_string(),
        }],
        context_json: "[]".to_string(),
        accessed_at_utc: "2026-07-23T12:00:00.000Z".to_string(),
        retrieval_elapsed_ms: 12,
        dom_page_count: 0,
        headless_fallback_count: 0,
        degraded: false,
        error_code: None,
        error: None,
        receipt_digest: None,
        invocation_index: None,
        security: crate::sovereign_search::SovereignSearchSecurity {
            api_key_required: false,
            cookies_enabled: false,
            browser_automation_enabled: false,
            visible_browser_opened: false,
            proxy_environment_enabled: false,
            endpoint_allowlist: vec!["duckduckgo.com".to_string()],
        },
    };

    let command_response = search_response_to_command_response(response);
    assert!(matches!(command_response.status, CommandStatus::Completed));
    assert!(command_response.claims.iter().any(|claim| {
        claim.contains("engine=duckduckgo_lite_static")
            && claim.contains("result_count=1")
            && claim.contains("degraded=false")
    }));
    assert!(command_response
        .claims
        .iter()
        .all(|claim| !claim.contains("relevance_score=")));
}

#[test]
fn degraded_duckduckgo_response_is_failed_and_not_signable() {
    let response = crate::sovereign_search::SovereignSearchResponse {
        query: "current facts".to_string(),
        engine: "duckduckgo_lite_static".to_string(),
        result_count: 0,
        results: Vec::new(),
        context_json: "[]".to_string(),
        accessed_at_utc: "2026-07-23T12:00:00.000Z".to_string(),
        retrieval_elapsed_ms: 12,
        dom_page_count: 0,
        headless_fallback_count: 0,
        degraded: true,
        error_code: Some("search_unavailable".to_string()),
        error: Some("network unavailable".to_string()),
        receipt_digest: None,
        invocation_index: None,
        security: crate::sovereign_search::SovereignSearchSecurity {
            api_key_required: false,
            cookies_enabled: false,
            browser_automation_enabled: false,
            visible_browser_opened: false,
            proxy_environment_enabled: false,
            endpoint_allowlist: vec!["duckduckgo.com".to_string()],
        },
    };
    let command_response = search_response_to_command_response(response);
    assert!(matches!(command_response.status, CommandStatus::Failed));
    let error = validate_signable_tool_response(&command_response)
        .expect_err("failed search cannot be certified");
    assert_eq!(error.code, "tool_execution_failed");
}

mod classifier_routing;
mod classifier_routing_cadence;
mod continuation;
mod planner_fallback_regressions;
mod planner_specialist_routing;
