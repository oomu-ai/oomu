use super::super::*;

pub(super) fn domain_error(
    code: &'static str,
    boundary: &'static str,
    message: impl Into<String>,
) -> AgenticLoopError {
    AgenticLoopError {
        code,
        boundary,
        message: message.into(),
        mlc_path: None,
    }
}

pub(super) fn verify_baseline_locked(
    agent_manager: &crate::agent_manager::AgentManager,
    request: &AutoRouteSessionBaselineRequest,
    model_root: &std::path::Path,
) -> Result<VerifiedAutoRouteBaseline, AgenticLoopError> {
    let providers = agent_manager
        .select_provider_configs_metadata_locked()
        .map_err(|error| {
            domain_error(
                "auto_route_provider_store_unavailable",
                "auto_route_provider_identity",
                error.to_string(),
            )
        })?;
    super::super::auto_route_validation::resolve_verified_auto_route_baseline(
        &providers, request, model_root,
    )
    .map_err(|error| domain_error(error.code, error.boundary, error.message))
}

fn emit_rolled_back_activation(
    engine: &PersistenceEngine,
    session_id: &str,
    baseline: Option<&AutoRouteSessionBaselineRequest>,
    error_code: &'static str,
    retryable: bool,
    previous: &super::super::auto_route::AutoRoutePersistedStateEvidence,
) {
    let Ok(current) = super::super::auto_route::read_persisted_auto_route_state(engine, session_id)
    else {
        crate::diagnostic_output::write_diagnostic_line(format_args!(
            "AUTO_ROUTE_ROLLBACK_STATE_UNAVAILABLE session={}",
            crate::redaction::redacted_log_text(session_id)
        ));
        return;
    };
    let unchanged = previous.state_digest == current.state_digest
        && previous.route_generation == current.route_generation;
    let receipt = rolled_back_activation_receipt(
        session_id, baseline, error_code, retryable, previous, current, unchanged,
    );
    super::super::auto_route::emit_auto_route_receipt(&receipt);
}

fn rolled_back_activation_receipt(
    session_id: &str,
    baseline: Option<&AutoRouteSessionBaselineRequest>,
    error_code: &'static str,
    retryable: bool,
    previous: &super::super::auto_route::AutoRoutePersistedStateEvidence,
    current: super::super::auto_route::AutoRoutePersistedStateEvidence,
    unchanged: bool,
) -> AutoRouteActivationReceipt {
    let now = unix_time_ms();
    AutoRouteActivationReceipt {
        kind: "auto_route_activation",
        receipt_id: format!("auto-route-failed-{}-{now}", session_id.trim()),
        session_id: session_id.trim().to_string(),
        provider_config_id: baseline.map(|value| value.provider_config_id.clone()),
        provider_type: baseline.map(|value| value.provider_type.clone()),
        model_id: baseline.map(|value| value.model_id.clone()),
        provenance: baseline.map(|_| AutoRouteProvenance::ExplicitSession),
        previous_route_generation: previous.route_generation,
        current_route_generation: current.route_generation,
        previous_state_digest: previous.state_digest.clone(),
        current_state_digest: current.state_digest,
        dynamic_routing_enabled: current.dynamic_routing_enabled,
        changed: !unchanged,
        committed: false,
        rolled_back: unchanged,
        retryable,
        error_code: Some(error_code),
    }
}

#[tauri::command]
pub async fn update_chat_session_dynamic_routing_override(
    session_id: String,
    dynamic_routing_override: Option<bool>,
    auto_route_baseline: Option<AutoRouteSessionBaselineRequest>,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    agent_manager: tauri::State<'_, crate::agent_manager::AgentManager>,
) -> Result<AutoRouteActivationResponse, AgenticLoopError> {
    let model_root = crate::settings::resolved_local_model_directory(&app)
        .map_err(AgenticLoopError::from_persistence)?;
    let engine = persistence.inner().clone();
    let agent_manager = agent_manager.inner().clone();
    let engine_for_join_failure = engine.clone();
    let session_for_worker = session_id.clone();
    let baseline_for_worker = auto_route_baseline.clone();
    let before_for_join = std::sync::Arc::new(std::sync::Mutex::new(None));
    let before_for_worker = std::sync::Arc::clone(&before_for_join);

    tauri::async_runtime::spawn_blocking(move || {
        // Provider identity and persisted route state must remain frozen across
        // validation and commit in both directions. Otherwise disabling
        // Auto-route can restore a provider or model that was deleted while
        // the toggle was in flight.
        let _provider_guard = agent_manager.lock_writes();
        let _persistence_guard = engine.lock_writes();
        let previous =
            super::super::auto_route::read_persisted_auto_route_state(&engine, &session_id)
                .map_err(|error| {
                    domain_error(
                        "auto_route_activation_state_unavailable",
                        "auto_route_activation",
                        error.to_string(),
                    )
                })?;
        if let Ok(mut slot) = before_for_worker.lock() {
            *slot = Some(previous.clone());
        }
        let baseline = if dynamic_routing_override == Some(true) {
            auto_route_baseline.clone().ok_or_else(|| {
                emit_rolled_back_activation(
                    &engine,
                    &session_id,
                    None,
                    "auto_route_baseline_incomplete",
                    false,
                    &previous,
                );
                domain_error(
                    "auto_route_baseline_incomplete",
                    "auto_route_provider_identity",
                    "Choose an on-device model before turning on Auto-route.",
                )
            })?
        } else {
            engine
                .saved_auto_route_baseline_request_locked(&session_id)
                .map_err(|error| {
                    emit_rolled_back_activation(
                        &engine,
                        &session_id,
                        None,
                        "auto_route_baseline_incomplete",
                        false,
                        &previous,
                    );
                    domain_error(
                        "auto_route_baseline_incomplete",
                        "auto_route_provider_identity",
                        error.to_string(),
                    )
                })?
        };
        let verified_baseline = {
            let verified = verify_baseline_locked(&agent_manager, &baseline, &model_root).map_err(
                |error| {
                    let retryable = matches!(
                        error.code,
                        "auto_route_provider_store_unavailable"
                            | "auto_route_provider_configuration_missing"
                            | "auto_route_provider_identity_mismatch"
                            | "auto_route_provider_model_mismatch"
                            | "auto_route_local_model_store_unavailable"
                            | "local_model_not_installed"
                            | "local_model_artifact_missing"
                    );
                    emit_rolled_back_activation(
                        &engine,
                        &session_id,
                        Some(&baseline),
                        error.code,
                        retryable,
                        &previous,
                    );
                    error
                },
            )?;
            Some(verified)
        };

        engine
            .update_chat_session_dynamic_routing_override_locked(
                &session_id,
                dynamic_routing_override,
                verified_baseline,
                Some(&model_root),
            )
            .map_err(|error| {
                emit_rolled_back_activation(
                    &engine,
                    &session_id,
                    Some(&baseline),
                    "auto_route_activation_persistence_failed",
                    true,
                    &previous,
                );
                domain_error(
                    "auto_route_activation_persistence_failed",
                    "auto_route_activation",
                    error.to_string(),
                )
            })
    })
    .await
    .map_err(|error| {
        let _persistence_guard = engine_for_join_failure.lock_writes();
        if let Ok(slot) = before_for_join.lock() {
            if let Some(previous) = slot.as_ref() {
                emit_rolled_back_activation(
                    &engine_for_join_failure,
                    &session_for_worker,
                    baseline_for_worker.as_ref(),
                    "auto_route_activation_worker_failed",
                    true,
                    previous,
                );
            }
        }
        domain_error(
            "auto_route_activation_worker_failed",
            "auto_route_activation",
            error.to_string(),
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PROVIDER_CONFIG_ID: &str = "prov-auto-route-disable-test";

    fn model_root() -> std::path::PathBuf {
        crate::db::tests::test_local_models::root()
    }

    fn temporary_paths(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let nonce = crate::foundation::clock::unix_time_ns_u128();
        let root = std::env::temp_dir().join(format!(
            "oomu-auto-route-disable-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("temporary root");
        (root.join("state.sqlite"), root.join("agents.sqlite"))
    }

    fn local_provider(
        provider_type: &str,
        model_id: &str,
    ) -> crate::agent_manager::ConfiguredProvider {
        crate::agent_manager::ConfiguredProvider {
            id: TEST_PROVIDER_CONFIG_ID.to_string(),
            provider_id: provider_type.to_string(),
            provider_name: "On-device".to_string(),
            auth_method: "custom".to_string(),
            base_url: String::new(),
            api_key_label: String::new(),
            api_key: None,
            credential_configured: false,
            custom_model_ids: model_id.to_string(),
            auto_route_target: false,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn explicit_baseline(model_id: &str) -> VerifiedAutoRouteBaseline {
        VerifiedAutoRouteBaseline {
            provider_config_id: ProviderConfigurationId::try_from(
                TEST_PROVIDER_CONFIG_ID.to_string(),
            )
            .expect("provider config ID"),
            provider_type: ProviderTypeId::try_from("local_model".to_string())
                .expect("provider type"),
            model_id: CanonicalModelId::try_from(model_id.to_string()).expect("model ID"),
            reasoning_depth: "medium".to_string(),
            context_budget: 12_288,
            provenance: AutoRouteProvenance::ExplicitSession,
        }
    }

    fn dynamic_session(engine: &PersistenceEngine, model_id: &str) -> ChatSessionRecord {
        engine
            .ensure_chat_session_with_auto_route_baseline(
                CreateChatSessionRequest {
                    agent_id: "agent-auto-route-disable".to_string(),
                    provider_id: "dynamic".to_string(),
                    model_id: "dynamic".to_string(),
                    title: Some("Atomic disable".to_string()),
                    dynamic_routing_override: Some(true),
                    workspace_id: None,
                },
                explicit_baseline(model_id),
                &model_root(),
            )
            .expect("dynamic session")
    }

    fn disable_with_exact_store_identity(
        engine: &PersistenceEngine,
        manager: &crate::agent_manager::AgentManager,
        session_id: &str,
        root: &std::path::Path,
    ) -> Result<AutoRouteActivationResponse, AgenticLoopError> {
        let _provider_guard = manager.lock_writes();
        let _persistence_guard = engine.lock_writes();
        let request = engine
            .saved_auto_route_baseline_request_locked(session_id)
            .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
        let verified = verify_baseline_locked(manager, &request, root)?;
        engine
            .update_chat_session_dynamic_routing_override_locked(
                session_id,
                Some(false),
                Some(verified),
                Some(root),
            )
            .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
    }

    fn assert_auto_route_remains_enabled(engine: &PersistenceEngine, session_id: &str) {
        let state =
            super::super::super::auto_route::read_persisted_auto_route_state(engine, session_id)
                .expect("persisted state");
        assert!(state.dynamic_routing_enabled);
        let session = engine
            .select_chat_sessions()
            .expect("sessions read")
            .into_iter()
            .find(|session| session.id == session_id)
            .expect("session exists");
        assert_eq!(session.provider_id, "dynamic");
        assert_eq!(session.model_id, "dynamic");
        assert_eq!(session.dynamic_routing_override, Some(true));
    }

    fn evidence(
        digest_byte: char,
        enabled: bool,
    ) -> super::super::super::auto_route::AutoRoutePersistedStateEvidence {
        super::super::super::auto_route::AutoRoutePersistedStateEvidence {
            state_digest: digest_byte.to_string().repeat(64),
            route_generation: RouteGeneration::verified(8).unwrap(),
            dynamic_routing_enabled: enabled,
        }
    }

    #[test]
    fn failed_enable_reports_the_unchanged_persisted_disabled_state() {
        let previous = evidence('a', false);
        let current = evidence('a', false);
        let receipt = rolled_back_activation_receipt(
            "session-rollback",
            None,
            "auto_route_local_provider_required",
            false,
            &previous,
            current,
            true,
        );
        assert!(!receipt.dynamic_routing_enabled);
        assert_eq!(receipt.previous_state_digest, receipt.current_state_digest);
        assert!(receipt.rolled_back);
        assert!(!receipt.changed);
    }

    #[test]
    fn failed_command_never_claims_rollback_when_persisted_state_changed() {
        let previous = evidence('a', false);
        let current = evidence('b', true);
        let receipt = rolled_back_activation_receipt(
            "session-changed",
            None,
            "auto_route_activation_worker_failed",
            true,
            &previous,
            current,
            false,
        );
        assert!(receipt.dynamic_routing_enabled);
        assert_ne!(receipt.previous_state_digest, receipt.current_state_digest);
        assert!(!receipt.rolled_back);
        assert!(receipt.changed);
    }

    #[test]
    fn disable_rejects_a_deleted_provider_and_keeps_auto_route_on() {
        let (state_path, manager_path) = temporary_paths("provider-deleted");
        let engine = PersistenceEngine::initialize_at(state_path).expect("persistence");
        let manager = crate::agent_manager::AgentManager::initialize_at(manager_path)
            .expect("provider store");
        let session = dynamic_session(&engine, crate::gemma::GEMMA_E4B_CANONICAL_ID);

        let error =
            disable_with_exact_store_identity(&engine, &manager, &session.id, &model_root())
                .expect_err("deleted provider blocks disable");

        assert_eq!(error.code, "auto_route_provider_configuration_missing");
        assert_auto_route_remains_enabled(&engine, &session.id);
    }

    #[test]
    fn disable_rejects_a_replaced_provider_type_and_keeps_auto_route_on() {
        let (state_path, manager_path) = temporary_paths("provider-replaced");
        let engine = PersistenceEngine::initialize_at(state_path).expect("persistence");
        let manager = crate::agent_manager::AgentManager::initialize_at(manager_path)
            .expect("provider store");
        manager
            .upsert_provider_config(local_provider(
                "local_gemma",
                crate::gemma::GEMMA_E4B_CANONICAL_ID,
            ))
            .expect("replacement provider");
        let session = dynamic_session(&engine, crate::gemma::GEMMA_E4B_CANONICAL_ID);

        let error =
            disable_with_exact_store_identity(&engine, &manager, &session.id, &model_root())
                .expect_err("provider type mismatch blocks disable");

        assert_eq!(error.code, "auto_route_provider_identity_mismatch");
        assert_auto_route_remains_enabled(&engine, &session.id);
    }

    #[test]
    fn disable_rejects_a_missing_model_and_keeps_auto_route_on() {
        let (state_path, manager_path) = temporary_paths("model-missing");
        let engine = PersistenceEngine::initialize_at(state_path).expect("persistence");
        let manager = crate::agent_manager::AgentManager::initialize_at(manager_path)
            .expect("provider store");
        manager
            .upsert_provider_config(local_provider(
                "local_model",
                crate::gemma::GEMMA_E4B_CANONICAL_ID,
            ))
            .expect("local provider");
        let session = dynamic_session(&engine, crate::gemma::GEMMA_E4B_CANONICAL_ID);
        let empty_root = std::env::temp_dir().join(format!(
            "oomu-empty-model-root-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        std::fs::create_dir_all(&empty_root).expect("empty model root");

        disable_with_exact_store_identity(&engine, &manager, &session.id, &empty_root)
            .expect_err("missing model blocks disable");

        assert_auto_route_remains_enabled(&engine, &session.id);
    }

    #[test]
    fn disable_restores_the_verified_explicit_e4b_route() {
        let (state_path, manager_path) = temporary_paths("success-e4b");
        let engine = PersistenceEngine::initialize_at(state_path).expect("persistence");
        let manager = crate::agent_manager::AgentManager::initialize_at(manager_path)
            .expect("provider store");
        manager
            .upsert_provider_config(local_provider(
                "local_model",
                crate::gemma::GEMMA_E4B_CANONICAL_ID,
            ))
            .expect("local provider");
        let session = dynamic_session(&engine, crate::gemma::GEMMA_E4B_CANONICAL_ID);

        let disabled =
            disable_with_exact_store_identity(&engine, &manager, &session.id, &model_root())
                .expect("verified disable succeeds");

        assert_eq!(disabled.session.provider_id, TEST_PROVIDER_CONFIG_ID);
        assert_eq!(
            disabled.session.model_id,
            crate::gemma::GEMMA_E4B_CANONICAL_ID
        );
        assert_eq!(disabled.session.dynamic_routing_override, Some(false));
        assert!(!disabled.receipt.dynamic_routing_enabled);
        assert!(disabled.receipt.committed);
    }
}
