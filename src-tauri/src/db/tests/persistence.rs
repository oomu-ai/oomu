use super::*;

#[test]
fn completed_state_ledger_refuses_a_missing_operations_database() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_missing_ops_db_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let state_path = temp_dir.join("state.sqlite");
    let engine = PersistenceEngine::initialize_at(state_path.clone()).unwrap();
    drop(engine);
    let operations_path = temp_dir.join(OPS_DB_FILE);
    remove_sqlite_sidecars(&operations_path);
    std::fs::remove_file(&operations_path).unwrap();

    let error = match PersistenceEngine::initialize_at(state_path) {
        Ok(_) => panic!("missing operations database must fail closed"),
        Err(error) => error,
    };
    assert!(
        error.contains("operations_store_metadata") || error.contains("operations database"),
        "unexpected error: {error}"
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn context_horizon_uses_typed_provider_semantics_for_local_and_cloud_budgets() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_context_horizon_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let content = "a".repeat(8_000);
    engine
        .insert_chat_message("session-a", "agent-a", "user", &content)
        .unwrap();

    engine
        .upsert_session_config(
            "session-a",
            "medium",
            16_384,
            Some("local-config-id"),
            Some("local_model"),
            Some("gemma-4-E2B-it-qat-q4_0-gguf"),
        )
        .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE active_session_configs SET provider_id='anthropic' WHERE session_id='session-a'",
            [],
        )
        .unwrap();
    let local = engine.session_context_status("session-a").unwrap();
    assert_eq!(local.estimated_tokens_used, 2_000);
    assert_eq!(local.tokens_total, 16_384);
    assert_eq!(local.working_budget_tokens, 16_384);
    assert_eq!(local.provider_max_tokens, 16_384);
    assert!((local.estimated_percentage_used - (2_000_f32 / 16_384_f32)).abs() < 0.000_001);
    assert!(!local.is_cloud_model);

    let unconfigured_local = engine
        .session_context_status("session-unconfigured")
        .unwrap();
    assert_eq!(
        unconfigured_local.tokens_total,
        settings::DEFAULT_CONTEXT_BUDGET
    );
    assert!(!unconfigured_local.is_cloud_model);

    engine
        .upsert_session_config(
            "session-a",
            "medium",
            4_096,
            Some("cloud-config-id"),
            Some("anthropic"),
            Some("claude-fable-5"),
        )
        .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE active_session_configs SET provider_id='local_model' WHERE session_id='session-a'",
            [],
        )
        .unwrap();
    let cloud = engine.session_context_status("session-a").unwrap();
    assert_eq!(cloud.estimated_tokens_used, 2_000);
    assert_eq!(cloud.tokens_total, 204_800);
    assert_eq!(cloud.working_budget_tokens, 4_096);
    assert_eq!(cloud.provider_max_tokens, 204_800);
    assert!(cloud.is_cloud_model);
    assert!(cloud.estimated_percentage_used > local.estimated_percentage_used);

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn persistence_engine_keeps_its_initial_workspace_namespace() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_stable_workspace_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let workspace_id = "00000000-0000-4000-8000-000000000216".to_string();
    let engine = PersistenceEngine {
        db_path: Arc::new(RwLock::new(temp_dir.join("state.sqlite"))),
        write_lock: Arc::new(Mutex::new(())),
        workspace_id: workspace_id.clone(),
        storage_class: Arc::new(RwLock::new(BackingStoreClass::Persistent)),
        ops_path: None,
    };
    engine.run_migrations().unwrap();

    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-stable".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Stable workspace".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    assert_eq!(session.workspace_id, workspace_id);

    engine
        .insert_chat_message(
            &session.id,
            &session.agent_id,
            "user",
            "Keep this session in its initial workspace namespace.",
        )
        .unwrap();
    assert_eq!(engine.select_chat_sessions().unwrap().len(), 1);
    assert_eq!(engine.select_chat_messages(&session.id).unwrap().len(), 1);

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn held_immediate_transaction_exposes_real_locked_database_failure() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_locked_db_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("state.sqlite");
    let engine = PersistenceEngine::initialize_at(db_path.clone()).unwrap();
    let lock_holder = engine.open_connection().unwrap();
    lock_holder.execute_batch("BEGIN IMMEDIATE;").unwrap();

    let key = get_database_key().unwrap();
    let contender = open_sqlcipher_database_connection_with_key(&db_path, &key).unwrap();
    contender.busy_timeout(Duration::from_millis(25)).unwrap();
    let error = contender
            .execute(
                "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('locked-canary', 'blocked', ?1, ?2)",
                params![unix_time_ms(), get_current_encryption_state()],
            )
            .unwrap_err()
            .to_string();
    assert!(error.to_ascii_lowercase().contains("locked") || error.contains("busy"));
    lock_holder.execute_batch("ROLLBACK;").unwrap();
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[cfg(unix)]
#[test]
fn read_only_database_and_directory_reject_writes() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = std::env::temp_dir().join(format!("oomu_read_only_db_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("state.sqlite");
    let engine = PersistenceEngine::initialize_at(db_path.clone()).unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    drop(connection);
    drop(engine);
    remove_sqlite_sidecars(&db_path);
    std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    std::fs::set_permissions(&temp_dir, std::fs::Permissions::from_mode(0o500)).unwrap();

    let key = get_database_key().unwrap();
    let write_result = open_sqlcipher_database_connection_with_key(&db_path, &key).and_then(
            |connection| {
                connection.execute(
                    "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('read-only-canary', 'blocked', ?1, ?2)",
                    params![unix_time_ms(), get_current_encryption_state()],
                )
            },
        );

    std::fs::set_permissions(&temp_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(write_result.is_err());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[cfg(unix)]
#[test]
fn unavailable_directory_permissions_fail_closed_without_database_creation() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = std::env::temp_dir().join(format!("oomu_unavailable_dir_{}", unix_time_ms()));
    let unavailable = temp_dir.join("unavailable");
    std::fs::create_dir_all(&unavailable).unwrap();
    std::fs::set_permissions(&unavailable, std::fs::Permissions::from_mode(0o000)).unwrap();
    let db_path = unavailable.join("state.sqlite");

    let result = PersistenceEngine::initialize_at(db_path.clone());

    std::fs::set_permissions(&unavailable, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert!(!db_path.exists());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn real_sqlite_full_condition_rolls_back_the_failed_write() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_sqlite_full_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    let page_count: i64 = connection
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    let page_size_value: rusqlite::types::Value = connection
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    let page_size = match page_size_value {
        rusqlite::types::Value::Integer(value) => value,
        rusqlite::types::Value::Text(value) => value.parse::<i64>().unwrap(),
        other => panic!("unexpected SQLCipher page-size value: {other:?}"),
    };
    connection
        .pragma_update(None, "max_page_count", page_count + 1)
        .unwrap();

    let error = connection
        .execute(
            "
                INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state)
                VALUES ('full-volume-canary', zeroblob(?1), ?2, ?3)
                ",
            params![
                page_size * 8,
                unix_time_ms(),
                get_current_encryption_state()
            ],
        )
        .unwrap_err();
    assert!(matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DiskFull)
    ));
    let persisted: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM app_preferences WHERE key='full-volume-canary'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted, 0);
    let integrity: String = connection
        .pragma_query_value(None, "integrity_check", |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn volatile_reconciliation_materializes_and_verifies_durable_operations_store() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_reconcile_companion_{}", unix_time_ms()));
    let volatile_dir = temp_dir.join("volatile");
    let durable_dir = temp_dir.join("durable");
    std::fs::create_dir_all(&volatile_dir).unwrap();
    std::fs::create_dir_all(&durable_dir).unwrap();
    let volatile_path = volatile_dir.join("state.sqlite");
    let durable_path = durable_dir.join("state.sqlite");
    let volatile = PersistenceEngine::initialize_volatile_at(volatile_path).unwrap();
    volatile
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('volatile-companion-record', 'recover-me', ?1, ?2)",
                params![unix_time_ms(), get_current_encryption_state()],
            )
            .unwrap();

    let report = volatile
        .reconcile_volatile_store_to(durable_path.clone(), false)
        .unwrap();
    assert!(!report.requires_confirmation);
    assert!(report.durable_probe_verified);
    let durable_operations_path = durable_dir.join(OPS_DB_FILE);
    assert!(durable_operations_path.is_file());
    assert!(!has_plaintext_sqlite_header(&durable_operations_path));
    let key = get_database_key().unwrap();
    let operations =
        open_sqlcipher_database_connection_with_key(&durable_operations_path, &key).unwrap();
    verify_operations_database(&operations).unwrap();
    drop(operations);

    let restarted = PersistenceEngine::initialize_at(durable_path).unwrap();
    restarted.probe_active_durable_store().unwrap();
    let recovered: String = restarted
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT value FROM app_preferences WHERE key='volatile-companion-record'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recovered, "recover-me");
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn empty_volatile_reconciliation_preserves_nonempty_durable_store_without_overwrite() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_reconcile_empty_{}", unix_time_ms()));
    let volatile_dir = temp_dir.join("volatile");
    let durable_dir = temp_dir.join("durable");
    std::fs::create_dir_all(&volatile_dir).unwrap();
    std::fs::create_dir_all(&durable_dir).unwrap();
    let durable_path = durable_dir.join("state.sqlite");
    let durable = PersistenceEngine::initialize_at(durable_path.clone()).unwrap();
    durable
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('durable-only-record', 'must-survive', ?1, ?2)",
                params![unix_time_ms(), get_current_encryption_state()],
            )
            .unwrap();
    drop(durable);
    let durable_operations_path = durable_dir.join(OPS_DB_FILE);
    remove_sqlite_sidecars(&durable_operations_path);
    std::fs::remove_file(&durable_operations_path).unwrap();
    assert!(PersistenceEngine::initialize_at(durable_path.clone()).is_err());
    assert!(durable_operations_path.is_file());

    let volatile =
        PersistenceEngine::initialize_volatile_at(volatile_dir.join("state.sqlite")).unwrap();
    assert_eq!(
        count_recoverable_records(&volatile.open_connection().unwrap()).unwrap(),
        0
    );
    let report = volatile
        .reconcile_volatile_store_to(durable_path.clone(), false)
        .unwrap();
    assert!(!report.requires_confirmation);
    assert_eq!(report.recovered_records, 0);
    assert!(!report.backup_created);
    assert_eq!(volatile.storage_class(), BackingStoreClass::Persistent);

    let preserved: String = volatile
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT value FROM app_preferences WHERE key='durable-only-record'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(preserved, "must-survive");
    volatile.probe_active_durable_store().unwrap();
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn safe_mode_boot_rules_disable_dynamic_routing_overrides() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_safe_mode_routes_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("safe-mode.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: Some("Safe mode route".to_string()),
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
    engine
        .update_chat_session_dynamic_routing_override(
            &session.id,
            Some(true),
            Some(test_verified_auto_route_baseline(
                crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
            )),
            Some(&installed_model_root()),
        )
        .unwrap();

    engine.apply_safe_mode_boot_rules().unwrap();

    let selected = engine.select_chat_session_by_id(&session.id).unwrap();
    assert_eq!(selected.dynamic_routing_override, Some(false));
    assert_eq!(selected.provider_id, "dynamic");
    assert_eq!(selected.model_id, "dynamic");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sovereign_ledger_stats_aggregate_local_cloud_and_savings() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_sovereign_ledger_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "openai".to_string(),
            model_id: "gpt-5.5".to_string(),
            title: Some("Ledger".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();

    engine
        .insert_chat_message_with_metadata(
            &session.id,
            "agent-test",
            "assistant",
            "Local answer.",
            Some("local_model"),
            Some("gemma-4-E2B-it-qat-q4_0-gguf"),
            Some(&json!({
                "routingMode": "dynamic",
                "eventKind": "dynamic_routing",
                "executingProviderId": "local_model",
                "executingModelId": "gemma-4-E2B-it-qat-q4_0-gguf",
                "promptTokens": 1200,
                "completionTokens": 300,
            })),
        )
        .unwrap();
    engine
        .insert_chat_message_with_metadata(
            &session.id,
            "agent-test",
            "assistant",
            "Dynamic cloud answer.",
            Some("openai"),
            Some("gpt-5.5"),
            Some(&json!({
                "routingMode": "dynamic",
                "eventKind": "dynamic_routing",
                "executingProviderId": "openai",
                "executingModelId": "gpt-5.5",
            })),
        )
        .unwrap();

    engine
        .insert_chat_message_with_metadata(
            &session.id,
            "agent-test",
            "assistant",
            "Static cloud answer.",
            Some("openai"),
            Some("gpt-5.5"),
            Some(&json!({
                "routingMode": "static",
                "executingProviderId": "openai",
                "executingModelId": "gpt-5.5",
            })),
        )
        .unwrap();

    let stats = engine.sovereign_ledger_stats(None).unwrap();
    assert_eq!(stats.total_local_turns, 1);
    assert_eq!(stats.total_cloud_turns, 2);
    assert_eq!(stats.protected_input_tokens, 1200);
    assert_eq!(stats.protected_output_tokens, 300);
    assert!((stats.ratio_on_device - 33.333).abs() < 0.01);
    assert!((stats.estimated_api_savings - 0.003).abs() < f64::EPSILON);
    assert!(stats.estimated_api_savings > 0.0);
    assert!(stats.data_egress_protected_mb > 0.0);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sovereign_ledger_stats_respect_since_filter_and_reset_cutoff() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_sovereign_ledger_filter_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "openai".to_string(),
            model_id: "gpt-5.5".to_string(),
            title: Some("Ledger filter".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let old_ms = 1_700_000_000_000_i64;
    let new_ms = old_ms + 10_000;

    let old_message_id = engine
        .insert_chat_message_with_metadata(
            &session.id,
            "agent-test",
            "assistant",
            "Local historical answer.",
            Some("local_model"),
            Some("gemma-4-E2B-it-qat-q4_0-gguf"),
            Some(&json!({
                "input_tokens_estimate": 100,
                "output_tokens_estimate": 50,
                "executingProviderId": "local_model",
                "executingModelId": "gemma-4-E2B-it-qat-q4_0-gguf",
            })),
        )
        .unwrap();
    let new_message_id = engine
        .insert_chat_message_with_metadata(
            &session.id,
            "agent-test",
            "assistant",
            "Cloud current answer.",
            Some("openai"),
            Some("gpt-5.5"),
            Some(&json!({
                "routingMode": "static",
                "executingProviderId": "openai",
                "executingModelId": "gpt-5.5",
            })),
        )
        .unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE chat_messages SET timestamp_ms = ?1 WHERE id = ?2",
            params![old_ms, old_message_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE chat_messages SET timestamp_ms = ?1 WHERE id = ?2",
            params![new_ms, new_message_id],
        )
        .unwrap();
    drop(connection);

    let filtered = engine.sovereign_ledger_stats(Some(new_ms)).unwrap();
    assert_eq!(filtered.total_local_turns, 0);
    assert_eq!(filtered.total_cloud_turns, 1);

    engine
        .upsert_app_preference(LEDGER_RESET_AT_KEY, &(new_ms + 1).to_string())
        .unwrap();
    let reset_filtered = engine.sovereign_ledger_stats(Some(old_ms)).unwrap();
    assert_eq!(reset_filtered.total_local_turns, 0);
    assert_eq!(reset_filtered.total_cloud_turns, 0);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn routing_preferences_persist_structured_primary_and_fallback_routes() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_routing_preferences_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("routes.sqlite")).unwrap();

    engine
        .upsert_model_routing_preference(
            "fallback",
            "prov-3",
            Some("prov-3"),
            "gemini-3.5-flash",
            Some("Google Gemini / gemini-3.5-flash"),
        )
        .unwrap();
    let fallback = engine
        .select_routing_preference("oomu-fallback-route")
        .unwrap()
        .expect("fallback route should persist");
    assert_eq!(fallback.key, "oomu-fallback-route");
    assert_eq!(fallback.route_key.as_deref(), Some("fallback"));
    assert_eq!(fallback.provider_id.as_deref(), Some("prov-3"));
    assert_eq!(fallback.provider_config_id.as_deref(), Some("prov-3"));
    assert_eq!(fallback.model_id.as_deref(), Some("gemini-3.5-flash"));
    assert_eq!(
        fallback.label.as_deref(),
        Some("Google Gemini / gemini-3.5-flash")
    );

    let fallback_by_slot = engine
        .select_routing_preference("fallback")
        .unwrap()
        .expect("fallback slot alias should resolve");
    assert_eq!(fallback_by_slot.key, fallback.key);
    assert_eq!(fallback_by_slot.value, fallback.value);

    engine
            .upsert_routing_preference(
                "primary",
                r#"{"providerConfigId":"prov-1","providerId":"prov-1","modelId":"local-gemma","label":"Local / local-gemma"}"#,
            )
            .unwrap();
    let primary = engine
        .select_routing_preference("oomu-primary-route")
        .unwrap()
        .expect("legacy primary alias should resolve");
    assert_eq!(primary.key, "primary");
    assert_eq!(primary.route_key.as_deref(), Some("primary"));
    assert_eq!(primary.model_id.as_deref(), Some("local-gemma"));

    let global = engine
        .select_user_routing_preference("default")
        .unwrap()
        .expect("global routing preference should mirror structured slots");
    assert_eq!(
        global.fallback_route_id.as_deref(),
        Some("prov-3:gemini-3.5-flash")
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn user_routing_preference_pair_persists_primary_and_fallback_ids() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_user_routing_pair_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("routes.sqlite")).unwrap();

    engine
        .upsert_user_routing_preference_pair("default", "provider-a:model-a", "provider-b:model-b")
        .unwrap();

    let global = engine
        .select_user_routing_preference("default")
        .unwrap()
        .expect("global routing pair should persist");
    assert_eq!(
        global.primary_route_id.as_deref(),
        Some("provider-a:model-a")
    );
    assert_eq!(
        global.fallback_route_id.as_deref(),
        Some("provider-b:model-b")
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn app_preferences_persist_and_update_scalar_values() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_app_preferences_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("preferences.sqlite")).unwrap();

    assert_eq!(engine.select_app_preference("ui.locale").unwrap(), None);
    engine
        .upsert_app_preference("ui.locale", "en-US")
        .expect("locale preference inserts");
    assert_eq!(
        engine
            .select_app_preference("ui.locale")
            .unwrap()
            .as_deref(),
        Some("en-US"),
    );

    engine
        .upsert_app_preference("ui.locale", "test-TEST")
        .expect("locale preference updates");
    assert_eq!(
        engine
            .select_app_preference("ui.locale")
            .unwrap()
            .as_deref(),
        Some("test-TEST"),
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn mod_database_connection_is_namespaced_and_read_only() {
    let mod_id = format!("ai.eldris.mods.test-{}", unix_time_ms());
    let db_path = get_mod_db_path(&mod_id).unwrap();
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    {
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "
                    CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
                    INSERT INTO records (value) VALUES ('sealed');
                    ",
            )
            .unwrap();
    }

    let read_only = get_mod_db_connection(&mod_id).unwrap();
    let value: String = read_only
        .query_row("SELECT value FROM records WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(value, "sealed");
    assert!(read_only
        .execute("INSERT INTO records (value) VALUES ('mutated')", [])
        .is_err());
    assert!(matches!(
        get_mod_db_path("../escape"),
        Err(DatabaseError::InvalidModId(_))
    ));

    let mod_root = db_path.parent().unwrap().parent().unwrap().to_path_buf();
    let _ = std::fs::remove_dir_all(mod_root);
}
