use super::*;
use crate::agent_manager::AgentManager;
use crate::db::canonical_model_authority::AuthorityMigrationFailurePoint;
use std::path::{Path, PathBuf};

fn stores(label: &str) -> (PathBuf, PersistenceEngine, AgentManager) {
    let root = std::env::temp_dir().join(format!(
        "oomu-authority-{label}-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    std::fs::create_dir_all(&root).expect("temporary profile exists");
    let persistence = PersistenceEngine::initialize_at(root.join("oomu_state.sqlite"))
        .expect("state database opens");
    let agents =
        AgentManager::initialize_at(root.join("oomu_ops.db")).expect("agent database opens");
    open_ops_database_connection(&root.join("oomu_ops.db"))
        .expect("agent database opens")
        .execute(
            "INSERT INTO provider_configs (
                 id, provider_id, provider_name, auth_method, base_url, api_key_label,
                 api_key, credential_configured, custom_model_ids, auto_route_target,
                 created_at_ms, updated_at_ms
             ) VALUES (
                 'prov-local-authority-test', 'local_model', 'On-device', 'none', '', '',
                 NULL, 1, ?1, 0, 1, 1
             )",
            params![format!(
                "{},{}",
                crate::gemma::GEMMA_E2B_CANONICAL_ID,
                crate::gemma::GEMMA_E4B_CANONICAL_ID
            )],
        )
        .expect("local provider configuration saves");
    (root, persistence, agents)
}

fn startup(model_root: &Path) -> crate::gemma::StartupModelAssignment {
    crate::gemma::resolve_verified_startup_model_assignment(
        model_root,
        &crate::gemma::StartupModelPreference {
            requested_model_id: crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            selection_source: crate::gemma::StartupModelSelectionSource::CleanDefault,
        },
    )
    .expect("verified startup assignment exists")
}

fn insert_agent(path: &Path, agent_id: &str, model_id: &str) {
    let now = unix_time_ms();
    open_ops_database_connection(path)
        .expect("agent database opens")
        .execute(
            "INSERT INTO agent_configs (
                 id, name, system_prompt, model_id, provider_id, description,
                 image, personality_profile, status, created_at_ms, updated_at_ms
             ) VALUES (?1, 'OOMU', 'Be helpful.', ?2, 'local_model', '',
                       NULL, '{}', 'active', ?3, ?3)",
            params![agent_id, model_id, now],
        )
        .expect("agent fixture saves");
}

fn agent_model(path: &Path, agent_id: &str) -> String {
    open_ops_database_connection(path)
        .expect("agent database opens")
        .query_row(
            "SELECT model_id FROM agent_configs WHERE id = ?1",
            params![agent_id],
            |row| row.get(0),
        )
        .expect("agent model reads")
}

fn dynamic_session(
    persistence: &PersistenceEngine,
    agent_id: &str,
    model_id: &str,
) -> ChatSessionRecord {
    persistence
        .ensure_chat_session_with_auto_route_baseline(
            CreateChatSessionRequest {
                agent_id: agent_id.to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Authority migration".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            },
            VerifiedAutoRouteBaseline {
                provider_config_id: ProviderConfigurationId::try_from(
                    "prov-local-authority-test".to_string(),
                )
                .expect("provider configuration ID"),
                provider_type: ProviderTypeId::try_from("local_model".to_string())
                    .expect("provider type"),
                model_id: CanonicalModelId::try_from(model_id.to_string()).expect("model ID"),
                reasoning_depth: "medium".to_string(),
                context_budget: 12_288,
                provenance: AutoRouteProvenance::ExplicitSession,
            },
            &installed_model_root(),
        )
        .expect("dynamic session exists")
}

fn make_legacy_session(persistence: &PersistenceEngine, agent_id: &str) -> ChatSessionRecord {
    let session = dynamic_session(
        persistence,
        agent_id,
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
    );
    persistence
        .open_connection()
        .expect("state database opens")
        .execute(
            "UPDATE active_session_configs
             SET model_id = 'gemma-4-2b', local_model_source = 'legacy_unverified'
             WHERE session_id = ?1",
            params![session.id],
        )
        .expect("legacy session saves");
    session
}

fn erase_typed_provider_identity(persistence: &PersistenceEngine, session_id: &str) {
    persistence
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE active_session_configs
         SET local_provider_config_id=NULL, local_provider_type=NULL,
             local_route_generation=0
         WHERE session_id=?1",
            params![session_id],
        )
        .expect("legacy manual provider identity is intentionally partial");
    let policy = persistence
        .select_chat_session_route_policy(session_id)
        .unwrap()
        .unwrap();
    assert_eq!(policy.local_provider_id, None);
    assert_eq!(policy.local_provider_type, None);
}

fn assert_reopened_route_policies(
    reopened: &PersistenceEngine,
    legacy_id: &str,
    explicit_id: &str,
    manual_id: &str,
) {
    let legacy = reopened
        .select_chat_session_route_policy(legacy_id)
        .unwrap()
        .unwrap();
    let explicit = reopened
        .select_chat_session_route_policy(explicit_id)
        .unwrap()
        .unwrap();
    let manual = reopened
        .select_chat_session_route_policy(manual_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        legacy.local_model_id.as_deref(),
        Some(crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID)
    );
    assert_eq!(
        explicit.local_model_id.as_deref(),
        Some(crate::gemma::GEMMA_E4B_CANONICAL_ID)
    );
    assert_eq!(explicit.local_source.as_deref(), Some("explicit_session"));
    assert_eq!(
        manual.local_provider_id.as_deref(),
        Some("prov-local-authority-test")
    );
    assert_eq!(manual.local_provider_type.as_deref(), Some("local_model"));
    assert!(manual.route_generation > 0);
}

#[test]
fn failed_reconciliation_rolls_back_without_success_receipt_or_changed_rows() {
    for failure in [
        AuthorityMigrationFailurePoint::AfterAgentAlignment,
        AuthorityMigrationFailurePoint::AfterSessionAlignment,
    ] {
        let (root, persistence, agents) = stores("rollback");
        let ops_path = root.join("oomu_ops.db");
        insert_agent(&ops_path, "agent-legacy", "gemma-4-2b");
        let session = make_legacy_session(&persistence, "agent-legacy");
        let result = persistence.reconcile_canonical_model_authorities_with_failure(
            &agents,
            &installed_model_root(),
            &startup(&installed_model_root()),
            failure,
        );
        assert!(result.is_err());
        assert_eq!(agent_model(&ops_path, "agent-legacy"), "gemma-4-2b");
        let policy = persistence
            .select_chat_session_route_policy(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(policy.local_model_id.as_deref(), Some("gemma-4-2b"));
        assert_eq!(policy.local_source.as_deref(), Some("legacy_unverified"));
        let state_backups: i64 = persistence
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM auto_route_baseline_backups",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let agent_backups: i64 = open_ops_database_connection(&ops_path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM agent_model_assignment_backups",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((state_backups, agent_backups), (0, 0));
        drop((persistence, agents));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn cloud_chat_configuration_does_not_fail_local_model_authority_verification() {
    let (root, persistence, agents) = stores("cloud-chat");
    let ops_path = root.join("oomu_ops.db");
    open_ops_database_connection(&ops_path)
        .expect("agent database opens")
        .execute(
            "INSERT INTO provider_configs (
                 id, provider_id, provider_name, auth_method, base_url, api_key_label,
                 api_key, credential_configured, custom_model_ids, auto_route_target,
                 created_at_ms, updated_at_ms
             ) VALUES (
                 'prov-cloud-authority-test', 'zai', 'Z.AI', 'api_key',
                 'https://api.z.ai/api/paas/v4', '', NULL, 1, 'glm-5.2', 0, 1, 1
             )",
            [],
        )
        .expect("cloud provider configuration saves");
    let session = persistence
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-cloud".to_string(),
            provider_id: "prov-cloud-authority-test".to_string(),
            model_id: "glm-5.2".to_string(),
            title: Some("Cloud chat".to_string()),
            dynamic_routing_override: Some(false),
            workspace_id: None,
        })
        .expect("cloud chat exists");
    persistence
        .upsert_session_config(
            &session.id,
            "high",
            12_288,
            Some("prov-cloud-authority-test"),
            Some("zai"),
            Some("glm-5.2"),
        )
        .expect("cloud chat configuration saves");

    let report = persistence
        .reconcile_canonical_model_authorities(
            &agents,
            &installed_model_root(),
            &startup(&installed_model_root()),
        )
        .expect("a cloud chat is outside local-model authority verification");
    assert_eq!(report.sessions.inspected, 0);
    let policy = persistence
        .select_chat_session_route_policy(&session.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        policy.local_provider_id.as_deref(),
        Some("prov-cloud-authority-test")
    );
    assert_eq!(policy.local_provider_type.as_deref(), Some("zai"));
    assert_eq!(policy.local_model_id.as_deref(), Some("glm-5.2"));

    drop((persistence, agents));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn contaminated_manual_and_dynamic_routes_repair_without_changing_explicit_e4b() {
    let (root, persistence, agents) = stores("restart");
    let ops_path = root.join("oomu_ops.db");
    insert_agent(&ops_path, "agent-legacy", "gemma-4-2b");
    let legacy = make_legacy_session(&persistence, "agent-legacy");
    let explicit = dynamic_session(
        &persistence,
        "agent-explicit-e4b",
        crate::gemma::GEMMA_E4B_CANONICAL_ID,
    );
    let manual = persistence
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-manual".to_string(),
            provider_id: "local_model".to_string(),
            model_id: crate::gemma::GEMMA_E2B_CANONICAL_ID.to_string(),
            title: Some("Manual route".to_string()),
            dynamic_routing_override: Some(false),
            workspace_id: None,
        })
        .expect("manual session exists");
    persistence
        .upsert_session_config(
            &manual.id,
            "medium",
            12_288,
            Some("local_model"),
            Some("local_model"),
            Some(crate::gemma::GEMMA_E2B_CANONICAL_ID),
        )
        .expect("contaminated manual baseline saves");
    erase_typed_provider_identity(&persistence, &manual.id);
    persistence
        .accept_chat_turn(AcceptChatTurnRequest {
            turn_id: "turn-migration-integrity".to_string(),
            generation_token: "generation-migration-integrity".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-migration-integrity".to_string(),
            turn_kind: "root".to_string(),
            session_id: manual.id.clone(),
            agent_id: manual.agent_id.clone(),
            provider_id: "local_model".to_string(),
            model_id: crate::gemma::GEMMA_E2B_CANONICAL_ID.to_string(),
            message: "Preserve this exact migration content.".to_string(),
        })
        .expect("conversation evidence exists before migration");
    let original_route: (Option<String>, Option<String>, String, i32, String) = persistence
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT provider_id,model_id,reasoning_depth,context_budget,local_model_source
             FROM active_session_configs WHERE session_id=?1",
            params![manual.id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    let report = persistence
        .reconcile_canonical_model_authorities(
            &agents,
            &installed_model_root(),
            &startup(&installed_model_root()),
        )
        .expect("authority transaction commits");
    assert_eq!(report.aligned_agents, 1);
    assert!(report.sessions.migration_integrity_verified);
    let connection = persistence.open_connection().unwrap();
    let preserved_content: String = connection
        .query_row(
            "SELECT content FROM chat_messages
             WHERE session_id=?1 AND role='user'",
            params![manual.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(preserved_content, "Preserve this exact migration content.");
    let preserved_turn: (String, String, String) = connection
        .query_row(
            "SELECT generation_token,root_turn_id,turn_kind FROM chat_turns WHERE turn_id=?1",
            params!["turn-migration-integrity"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        preserved_turn,
        (
            "generation-migration-integrity".to_string(),
            "turn-migration-integrity".to_string(),
            "root".to_string()
        )
    );
    let backup_route: (Option<String>, Option<String>, String, i32, String) = connection
        .query_row(
            "SELECT provider_id,model_id,reasoning_depth,context_budget,local_model_source
             FROM auto_route_baseline_backups WHERE session_id=?1",
            params![manual.id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(backup_route, original_route);
    drop(connection);
    assert_eq!(
        agent_model(&ops_path, "agent-legacy"),
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID
    );
    drop((persistence, agents));

    let reopened = PersistenceEngine::initialize_at(root.join("oomu_state.sqlite")).unwrap();
    let reopened_agents = AgentManager::initialize_at(ops_path.clone()).unwrap();
    assert_reopened_route_policies(&reopened, &legacy.id, &explicit.id, &manual.id);
    reopened
        .reconcile_canonical_model_authorities(
            &reopened_agents,
            &installed_model_root(),
            &startup(&installed_model_root()),
        )
        .expect("restart reconciliation is idempotent");
    drop((reopened, reopened_agents));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unavailable_manual_route_is_preserved_and_marked_for_one_user_choice() {
    let (root, persistence, agents) = stores("manual-choice");
    let ops_path = root.join("oomu_ops.db");
    insert_agent(
        &ops_path,
        "agent-manual-choice",
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
    );
    let session = persistence
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-manual-choice".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "missing-explicit-model".to_string(),
            title: Some("Unavailable manual route".to_string()),
            dynamic_routing_override: Some(false),
            workspace_id: None,
        })
        .expect("manual session exists");
    persistence
        .upsert_session_config(
            &session.id,
            "medium",
            12_288,
            Some("prov-local-authority-test"),
            Some("local_model"),
            Some("missing-explicit-model"),
        )
        .expect("unavailable manual baseline saves");

    let report = persistence
        .reconcile_canonical_model_authorities(
            &agents,
            &installed_model_root(),
            &startup(&installed_model_root()),
        )
        .expect("authority transaction commits");
    assert!(report.sessions.needs_user_choice >= 1);
    let policy = persistence
        .select_chat_session_route_policy(&session.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        policy.local_model_id.as_deref(),
        Some("missing-explicit-model")
    );
    assert_eq!(policy.local_source.as_deref(), Some("needs_user_choice"));
    assert!(policy.route_generation > 0);
    let backups: i64 = persistence
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM auto_route_baseline_backups WHERE session_id = ?1",
            params![session.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(backups, 1);
    drop((persistence, agents));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_authority_reconciliation_serializes_without_partial_state() {
    let (root, persistence, agents) = stores("concurrent");
    let ops_path = root.join("oomu_ops.db");
    insert_agent(&ops_path, "agent-legacy", "gemma-4-2b");
    let session = make_legacy_session(&persistence, "agent-legacy");
    let model_root = installed_model_root();
    let assignment = startup(&model_root);
    let first_engine = persistence.clone();
    let first_agents = agents.clone();
    let first_root = model_root.clone();
    let first_assignment = assignment.clone();
    let first = std::thread::spawn(move || {
        first_engine.reconcile_canonical_model_authorities(
            &first_agents,
            &first_root,
            &first_assignment,
        )
    });
    let second =
        persistence.reconcile_canonical_model_authorities(&agents, &model_root, &assignment);
    assert!(first.join().unwrap().is_ok());
    assert!(second.is_ok());
    assert_eq!(
        agent_model(&ops_path, "agent-legacy"),
        crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID
    );
    let policy = persistence
        .select_chat_session_route_policy(&session.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        policy.local_model_id.as_deref(),
        Some(crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID)
    );
    drop((persistence, agents));
    std::fs::remove_dir_all(root).unwrap();
}
