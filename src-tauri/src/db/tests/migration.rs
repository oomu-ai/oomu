use super::*;

#[test]
fn migration_ledger_records_exact_authoritative_source_checksums() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_migration_ledger_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();

    for migration in MIGRATIONS {
        let recorded: String = connection
            .query_row(
                "SELECT checksum_sha256 FROM schema_migration_ledger WHERE sequence=?1",
                params![migration.sequence],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recorded, migration_checksum(migration).unwrap());
        let modified_checksum = match migration.source {
            MigrationSource::Sql(source) => {
                hash_migration_material(&format!("{source}\n-- changed executable DDL"), &[])
            }
            MigrationSource::RustImplementation {
                contract,
                implementation_ids,
            } => {
                let mut implementations = implementation_ids
                    .iter()
                    .map(|implementation_id| {
                        (
                            *implementation_id,
                            migration_implementation_source(implementation_id).unwrap(),
                        )
                    })
                    .collect::<Vec<_>>();
                assert!(!implementations.is_empty());
                assert!(implementations
                    .iter()
                    .all(|(_, implementation)| !implementation.trim().is_empty()));
                let modified = format!(
                    "{}\n// changed executable migration behavior",
                    implementations[0].1
                );
                implementations[0].1 = &modified;
                hash_migration_material(contract, &implementations)
            }
            MigrationSource::HistoricalChecksum(checksum) => {
                assert_eq!(recorded, checksum);
                "0".repeat(64)
            }
        };
        assert_ne!(recorded, modified_checksum);
    }
    let completed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migration_ledger WHERE state='completed' AND completed_at_ms IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(completed, MIGRATIONS.len() as i64);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn truthful_background_migration_enforces_real_runtime_states() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_background_migration_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();

    connection
        .execute(
            "INSERT INTO background_service_state (singleton,user_enabled,service_status,updated_at_ms,requested_enabled,runtime_state,registration_state,process_state,build_number,profile_class,profile_generation,menu_visible) VALUES (1,0,'paused',1,0,'off','unregistered','absent',1,'test','profile-1',0)",
            [],
        )
        .unwrap();
    assert!(connection
        .execute(
            "UPDATE background_service_state SET runtime_state='active_without_heartbeat' WHERE singleton=1",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO background_runtime_receipts (receipt_id,event_kind,outcome,runtime_state,requested_enabled,build_number,profile_class,profile_generation,created_at_ms) VALUES ('receipt-1','unknown_event','verified','off',0,1,'test','profile-1',1)",
            [],
        )
        .is_err());

    drop(connection);
    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn connector_scope_migration_preserves_existing_bindings_and_marks_only_existing_accounts_reviewed()
{
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE connector_accounts (
           connector_id TEXT PRIMARY KEY,
           updated_at_ms INTEGER NOT NULL
         );
         CREATE TABLE connector_project_bindings (
           connector_id TEXT NOT NULL,
           project_id TEXT NOT NULL,
           enabled INTEGER NOT NULL,
           created_at_ms INTEGER NOT NULL,
           updated_at_ms INTEGER NOT NULL,
           PRIMARY KEY (connector_id, project_id)
         );
         INSERT INTO connector_accounts VALUES ('connector-a', 111);
         INSERT INTO connector_project_bindings VALUES ('connector-a','project-a',1,10,20);",
        )
        .unwrap();
    let before: (String, String, i64, i64, i64) = connection.query_row(
        "SELECT connector_id,project_id,enabled,created_at_ms,updated_at_ms FROM connector_project_bindings",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).unwrap();
    connector_scope_migration::apply(&connection).unwrap();
    let after: (String, String, i64, i64, i64) = connection.query_row(
        "SELECT connector_id,project_id,enabled,created_at_ms,updated_at_ms FROM connector_project_bindings",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).unwrap();
    assert_eq!(after, before);
    let existing: (i64, Option<i64>) = connection.query_row(
        "SELECT all_projects_enabled,project_scope_reviewed_at_ms FROM connector_accounts WHERE connector_id='connector-a'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(existing, (0, Some(111)));
    connection.execute("INSERT INTO connector_accounts (connector_id,updated_at_ms) VALUES ('connector-new',222)", []).unwrap();
    let created: (i64, Option<i64>) = connection.query_row(
        "SELECT all_projects_enabled,project_scope_reviewed_at_ms FROM connector_accounts WHERE connector_id='connector-new'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(created, (0, None));
}

#[test]
fn legacy_checksums_require_a_verified_schema_and_reject_unapproved_sql_migrations() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_legacy_runner_checksum_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    let historical_checksum = "a".repeat(64);

    connection
        .execute(
            "UPDATE schema_migration_ledger SET checksum_sha256=?1 WHERE sequence=3",
            params![historical_checksum],
        )
        .unwrap();
    drop(connection);
    engine.run_migrations().unwrap();

    let connection = engine.open_connection().unwrap();
    connection
        .execute_batch("DROP INDEX idx_workflow_blueprints_compilation_status;")
        .unwrap();
    drop(connection);
    let error = engine.run_migrations().unwrap_err().to_string();
    assert!(error.contains("idx_workflow_blueprints_compilation_status"));

    let connection = engine.open_connection().unwrap();
    connection
        .execute_batch(
            "CREATE INDEX idx_workflow_blueprints_compilation_status
             ON workflow_blueprints(compilation_status, updated_at_ms DESC);",
        )
        .unwrap();
    connection
        .execute(
            "UPDATE schema_migration_ledger SET checksum_sha256=?1 WHERE sequence=2",
            params!["b".repeat(64)],
        )
        .unwrap();
    drop(connection);
    let error = engine.run_migrations().unwrap_err().to_string();
    assert!(error.contains("checksum mismatch for 0002_workflow_execution"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn adaptive_learning_beta_checksum_recovers_only_with_the_verified_schema() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_adaptive_learning_checksum_{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE schema_migration_ledger SET checksum_sha256=?1 WHERE sequence=23",
            params!["c".repeat(64)],
        )
        .unwrap();
    engine.run_migrations().unwrap();

    engine
        .open_connection()
        .unwrap()
        .execute_batch("DROP TABLE saved_method_versions;")
        .unwrap();
    let error = engine.run_migrations().unwrap_err().to_string();
    assert!(error.contains("required table saved_method_versions is missing"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn agent_execution_origin_migration_retires_legacy_duplicates_idempotently() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_agent_execution_origin_upgrade_{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();

    connection
        .execute_batch(
            "
            DROP INDEX idx_agent_executions_active_plan_origin;
            ",
        )
        .unwrap();
    for (execution_id, status, created_at_ms, updated_at_ms) in [
        ("legacy-execution", "halted", 10_i64, 20_i64),
        ("newest-execution", "running", 30_i64, 40_i64),
    ] {
        connection
            .execute(
                "
                INSERT INTO agent_executions (
                    execution_id, plan_id, session_id, agent_id, provider_id, model_id,
                    turn_id, generation_token, parent_turn_id, root_turn_id, turn_kind,
                    context_json, status, created_at_ms, updated_at_ms, encryption_state
                ) VALUES (
                    ?1, 'shared-plan', 'session', 'agent', 'provider', 'model',
                    'shared-turn', 'shared-generation', NULL, 'shared-turn', 'root',
                    '{}', ?2, ?3, ?4, ?5
                )
                ",
                params![
                    execution_id,
                    status,
                    created_at_ms,
                    updated_at_ms,
                    get_current_encryption_state(),
                ],
            )
            .unwrap();
    }
    connection
        .execute_batch(AGENT_EXECUTION_ORIGIN_UNIQUENESS_MIGRATION)
        .unwrap();
    drop(connection);
    let upgraded = engine.open_connection().unwrap();
    let active_execution: String = upgraded
        .query_row(
            "SELECT execution_id FROM agent_executions
             WHERE plan_id='shared-plan' AND turn_id='shared-turn'
               AND generation_token='shared-generation'
               AND status IN ('running', 'halted')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_execution, "newest-execution");
    let retired_status: String = upgraded
        .query_row(
            "SELECT status FROM agent_executions WHERE execution_id='legacy-execution'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retired_status, "cancelled");
    let audit_count: i64 = upgraded
        .query_row(
            "SELECT COUNT(*) FROM agent_execution_logs
             WHERE execution_id='legacy-execution'
               AND json_extract(payload_json, '$.code')='duplicate_agent_execution_origin_retired'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count, 1);
    assert!(upgraded
        .query_row(
            "SELECT 1 FROM sqlite_master
             WHERE type='index' AND name='idx_agent_executions_active_plan_origin'",
            [],
            |_| Ok(()),
        )
        .is_ok());
    drop(upgraded);

    engine
        .open_connection()
        .unwrap()
        .execute_batch(AGENT_EXECUTION_ORIGIN_UNIQUENESS_MIGRATION)
        .unwrap();
    let audit_count_after_restart: i64 = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM agent_execution_logs
             WHERE execution_id='legacy-execution'
               AND json_extract(payload_json, '$.code')='duplicate_agent_execution_origin_retired'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count_after_restart, 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn current_operations_schema_is_minimal_and_versioned() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_operations_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let operations = engine.open_ops_connection().unwrap();
    verify_operations_database(&operations).unwrap();
    let version: String = operations
        .query_row(
            "SELECT value FROM operations_store_metadata WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "1");

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn conditional_migration_checksums_bind_runner_and_probe_source() {
    for (migration, implementation_id, behavior_probe) in [
        (
            MIGRATIONS[2],
            "0003_workflow_compilation_status",
            "add_column_if_missing",
        ),
        (
            MIGRATIONS[3],
            "0004_workflow_approval_gateway",
            "column_exists",
        ),
    ] {
        let MigrationSource::RustImplementation {
            contract,
            implementation_ids,
        } = migration.source
        else {
            panic!("conditional migration must bind its Rust runner");
        };
        assert_eq!(
            implementation_ids,
            &[implementation_id, "shared_schema_probes"]
        );

        let runner = migration_implementation_source(implementation_id).unwrap();
        let probes = migration_implementation_source("shared_schema_probes").unwrap();
        assert!(runner.contains(behavior_probe));
        assert!(probes.contains("fn add_column_if_missing"));
        assert!(probes.contains("fn column_exists"));

        let modified_runner = format!("{runner}\n// altered conditional behavior");
        let modified_checksum = hash_migration_material(
            contract,
            &[
                (implementation_id, modified_runner.as_str()),
                ("shared_schema_probes", probes),
            ],
        );
        assert_ne!(migration_checksum(migration).unwrap(), modified_checksum);
    }
}

#[test]
fn altered_migration_checksum_fails_closed() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_migration_checksum_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE schema_migration_ledger SET checksum_sha256=?1 WHERE sequence=2",
            params!["0".repeat(64)],
        )
        .unwrap();

    let error = engine.run_migrations().unwrap_err().to_string();
    assert!(error.contains("MIGRATION_RECOVERY_REQUIRED"));
    assert!(error.contains("checksum mismatch"));
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn partial_migration_ledger_state_fails_closed() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_migration_partial_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE schema_migration_ledger SET state='running', completed_at_ms=NULL WHERE sequence=5",
                [],
            )
            .unwrap();

    let error = engine.run_migrations().unwrap_err().to_string();
    assert!(error.contains("MIGRATION_RECOVERY_REQUIRED"));
    assert!(error.contains("partial migration ledger"));
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn out_of_order_or_unknown_migration_fails_closed() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_migration_order_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE schema_migration_ledger SET migration_id='9999_unknown' WHERE sequence=3",
            [],
        )
        .unwrap();

    let error = engine.run_migrations().unwrap_err().to_string();
    assert!(error.contains("MIGRATION_RECOVERY_REQUIRED"));
    assert!(error.contains("out-of-order"));
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn volatile_state_database_is_private_and_ciphertext_only() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_volatile_cipher_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let db_path = temp_dir.join("state.sqlite");
    let engine = PersistenceEngine::initialize_volatile_at(db_path.clone()).unwrap();
    let canary = "SPRINT218_VOLATILE_PLAINTEXT_CANARY";
    let connection = engine.open_connection().unwrap();
    connection
            .execute(
                "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES (?1, ?2, ?3, ?4)",
                params!["canary", canary, unix_time_ms(), get_current_encryption_state()],
            )
            .unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    drop(connection);

    let bytes = std::fs::read(&db_path).unwrap();
    assert!(!bytes
        .windows(canary.len())
        .any(|window| window == canary.as_bytes()));
    assert!(!has_plaintext_sqlite_header(&db_path));
    assert_eq!(engine.storage_class(), BackingStoreClass::Volatile);
    assert!(engine.require_durable_store("release signing").is_err());
    assert!(engine
        .insert_dynamic_routing_audit("prompt", "output", &json!({"route": "local"}))
        .is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&db_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn destructive_migration_backup_restores_and_passes_integrity() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_migration_restore_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    let backup_path: String = connection
            .query_row(
                "SELECT backup_path FROM schema_migration_ledger WHERE migration_id='0004_workflow_approval_gateway'",
                [],
                |row| row.get(0),
            )
            .unwrap();
    let backup_path = PathBuf::from(backup_path);
    assert!(backup_path.exists());
    assert!(!has_plaintext_sqlite_header(&backup_path));

    let restored_path = temp_dir.join("restored.sqlite");
    std::fs::copy(&backup_path, &restored_path).unwrap();
    let key = get_database_key().unwrap();
    let restored = open_sqlcipher_database_connection_with_key(&restored_path, &key).unwrap();
    let integrity: String = restored
        .pragma_query_value(None, "integrity_check", |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    verify_migration_ledger(&restored).unwrap();
    assert!(!column_exists(&restored, "execution_instances", "memory_json").unwrap());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn uncommitted_table_rebuild_transaction_rolls_back_without_ghost_schema() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_migration_interrupt_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    let backup_path: String = connection
        .query_row(
            "SELECT backup_path FROM schema_migration_ledger WHERE sequence=4",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let interrupted_path = temp_dir.join("interrupted.sqlite");
    std::fs::copy(backup_path, &interrupted_path).unwrap();
    let key = get_database_key().unwrap();
    let mut interrupted =
        open_sqlcipher_database_connection_with_key(&interrupted_path, &key).unwrap();
    interrupted
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    {
        let transaction = interrupted
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        transaction
                .execute_batch(
                    "ALTER TABLE execution_instances RENAME TO execution_instances_before_approval_gateway;",
                )
                .unwrap();
        // The runner's transaction boundary rolls back uncommitted DDL.
    }
    interrupted
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();

    assert!(table_exists(&interrupted, "execution_instances").unwrap());
    assert!(!table_exists(&interrupted, "execution_instances_before_approval_gateway").unwrap());
    let foreign_keys_enabled: i64 = interrupted
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys_enabled, 1);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn migration_process_crash_child() {
    let Ok(path) = std::env::var("OOMU_TEST_MIGRATION_CRASH_DB") else {
        return;
    };
    let key = get_database_key().unwrap();
    let connection = open_sqlcipher_database_connection_with_key(Path::new(&path), &key)
        .expect("child opens copied historical database");
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    connection
        .execute_batch(
            "BEGIN IMMEDIATE;
                 ALTER TABLE execution_instances
                 RENAME TO execution_instances_before_approval_gateway;",
        )
        .expect("child reaches destructive DDL boundary");
    std::process::exit(86);
}

#[test]
fn real_process_termination_recovers_without_ghost_schema() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_migration_process_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    let backup_path: String = connection
        .query_row(
            "SELECT backup_path FROM schema_migration_ledger WHERE sequence=4",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);
    let interrupted_path = temp_dir.join("process-interrupted.sqlite");
    std::fs::copy(backup_path, &interrupted_path).unwrap();

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("db::tests::migration::migration_process_crash_child")
        .arg("--nocapture")
        .env("OOMU_TEST_MIGRATION_CRASH_DB", &interrupted_path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));

    let key = get_database_key().unwrap();
    let recovered = open_sqlcipher_database_connection_with_key(&interrupted_path, &key).unwrap();
    recovered.pragma_update(None, "foreign_keys", "ON").unwrap();
    assert!(table_exists(&recovered, "execution_instances").unwrap());
    assert!(!table_exists(&recovered, "execution_instances_before_approval_gateway").unwrap());
    let violations: i64 = recovered
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    drop(recovered);

    let restarted = PersistenceEngine::initialize_at(interrupted_path).unwrap();
    let restarted_connection = restarted.open_connection().unwrap();
    verify_migration_ledger(&restarted_connection).unwrap();
    verify_schema_invariants(&restarted_connection, MIGRATIONS.len() as i64).unwrap();
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn channel_configs_schema_seeds_inactive_community_platforms() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_channels_seed_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    let configs = engine.select_channel_configs().unwrap();
    let platforms = configs
        .iter()
        .map(|config| config.platform.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        platforms,
        vec!["signal", "whatsapp", "telegram", "discord", "slack"]
    );
    assert!(configs.iter().all(|config| !config.is_active));

    let visible = engine.select_channel_config_summaries().unwrap();
    assert_eq!(
        visible
            .iter()
            .map(|config| config.platform.as_str())
            .collect::<Vec<_>>(),
        vec!["telegram", "discord", "slack"]
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn channel_summaries_do_not_hydrate_keychain_secrets() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_channel_summary_metadata_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    engine
        .upsert_channel_config(SaveChannelConfigRequest {
            platform: "telegram".to_string(),
            is_active: true,
            credentials_json: Some(r#"{"botToken":"summary-secret"}"#.to_string()),
            owner_id: Some("owner".to_string()),
        })
        .unwrap();
    crate::secret_store::evict_channel_secret_for_test("telegram");
    let reads_before = crate::secret_store::channel_secret_backend_reads_for_test("telegram");

    let summaries = engine.select_channel_config_summaries().unwrap();
    let telegram = summaries
        .iter()
        .find(|config| config.platform == "telegram")
        .unwrap();

    assert!(telegram.is_active);
    assert!(telegram.credential_configured);
    assert_eq!(
        crate::secret_store::channel_secret_backend_reads_for_test("telegram"),
        reads_before
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn chat_memory_keyword_search_returns_relevant_prior_blocks() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_chat_memory_search_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    engine
        .insert_chat_message(
            "session-a",
            "agent-a",
            "user",
            "We decided SQLite keyword retrieval powers the context engine.",
        )
        .unwrap();
    engine
        .insert_chat_message(
            "session-a",
            "agent-a",
            "assistant",
            "The unrelated color palette is teal and graphite.",
        )
        .unwrap();
    engine
        .insert_chat_message(
            "session-b",
            "agent-a",
            "user",
            "SQLite retrieval in another session should not appear.",
        )
        .unwrap();

    let current = "Please explain SQLite retrieval for the context engine.";
    engine
        .insert_chat_message("session-a", "agent-a", "user", current)
        .unwrap();

    let blocks = engine
        .search_relevant_chat_memory_blocks(Some("session-a"), "agent-a", current, Some(current), 3)
        .unwrap();

    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].content.contains("keyword retrieval"));
    assert_eq!(blocks[0].session_id, "session-a");
    assert!(!blocks[0].content.contains("Please explain"));

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sovereign_trust_schema_persists_global_policy_and_usage() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_trust_policy_{}", unix_time_ms()));
    let trusted_dir = temp_dir.join("trusted");
    std::fs::create_dir_all(&trusted_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    let connection = engine.open_connection().unwrap();
    for table in ["sovereign_trust_policies", "active_trust_sessions"] {
        let exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "{table} should be migrated");
    }
    drop(connection);

    let policy_id = engine
        .upsert_sovereign_trust_policy(
            trusted_dir.to_str().unwrap(),
            &[
                SovereignTrustToolCategory::ExternalWrites,
                SovereignTrustToolCategory::ShellCommands,
            ],
            SovereignTrustPermissionLevel::GlobalTrust,
            None,
            Some(128),
            Some(4.0),
        )
        .unwrap();
    assert!(policy_id > 0);

    let now_ms = unix_time_ms();
    let target = trusted_dir.join("drafts").join("note.md");
    let grant = engine
        .select_matching_sovereign_trust_grant(
            None,
            &target,
            SovereignTrustToolCategory::ExternalWrites,
            now_ms,
        )
        .unwrap()
        .expect("global trust policy should match nested write target");
    assert_eq!(
        grant.permission_level,
        SovereignTrustPermissionLevel::GlobalTrust
    );
    assert_eq!(grant.daily_token_cost_limit, 128);

    engine
        .record_sovereign_trust_usage(&grant, 7, 0.25, now_ms)
        .unwrap();
    let updated = engine
        .select_matching_sovereign_trust_grant(
            None,
            &target,
            SovereignTrustToolCategory::ExternalWrites,
            now_ms,
        )
        .unwrap()
        .expect("updated global trust policy should still match");
    assert_eq!(updated.token_cost_used_today, 7);
    assert!(updated.cpu_seconds_used_today >= 0.25);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_sqlite_sqlcipher_encryption_at_rest() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_test_db_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("test_encrypted.sqlite");

    // 1. Open with SQLCipher and encrypt
    {
        let conn = Connection::open(&db_path).unwrap();
        let key = get_database_key().unwrap();
        conn.pragma_update(None, "key", &key).unwrap();

        // Run standard setup
        conn.execute(
            "CREATE TABLE test_sec (id INTEGER PRIMARY KEY, secret TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO test_sec (secret) VALUES ('ELD_SECRET_CODENAME_SHIELD')",
            [],
        )
        .unwrap();
    }

    // 2. Verify that the file header is NOT standard SQLite header "SQLite format 3\0"
    {
        let mut file = std::fs::File::open(&db_path).unwrap();
        let mut header = [0u8; 16];
        file.read_exact(&mut header).unwrap();
        assert_ne!(
            &header, b"SQLite format 3\0",
            "Database file is unencrypted! Header contains SQLite format 3 magic string."
        );
    }

    // 3. Try to open without key and run query - MUST FAIL
    {
        let conn = Connection::open(&db_path).unwrap();
        let result = conn.execute("SELECT * FROM test_sec", []);
        assert!(
            result.is_err(),
            "Reading from database without a key should have failed, but succeeded!"
        );
    }

    // 4. Try to open with key and run query - MUST SUCCEED
    {
        let conn = Connection::open(&db_path).unwrap();
        let key = get_database_key().unwrap();
        conn.pragma_update(None, "key", &key).unwrap();
        let mut stmt = conn.prepare("SELECT secret FROM test_sec").unwrap();
        let mut rows = stmt.query([]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let secret: String = row.get(0).unwrap();
        assert_eq!(secret, "ELD_SECRET_CODENAME_SHIELD");
    }

    // Clean up
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn legacy_sqlcipher_database_is_rekeyed_to_argon2id_on_startup() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_legacy_sqlcipher_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("legacy_state.sqlite");
    let mut legacy_key = derive_legacy_bound_database_key("default_secure_test_key");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "key", &legacy_key).unwrap();
        conn.execute(
            "CREATE TABLE legacy_marker (id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO legacy_marker (value) VALUES ('old-key')", [])
            .unwrap();
    }

    let engine = PersistenceEngine::initialize_at(db_path.clone()).unwrap();
    let connection = engine.open_connection().unwrap();
    let value: String = connection
        .query_row("SELECT value FROM legacy_marker WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(value, "old-key");
    drop(connection);
    drop(engine);

    let legacy_connection = Connection::open(&db_path).unwrap();
    legacy_connection
        .pragma_update(None, "key", &legacy_key)
        .unwrap();
    assert!(legacy_connection
        .query_row("SELECT COUNT(*) FROM legacy_marker", [], |row| row
            .get::<_, i64>(0))
        .is_err());
    legacy_key.zeroize();

    let current_connection = Connection::open(&db_path).unwrap();
    let current_key = get_database_key().unwrap();
    current_connection
        .pragma_update(None, "key", &current_key)
        .unwrap();
    let count: i64 = current_connection
        .query_row("SELECT COUNT(*) FROM legacy_marker", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn plaintext_state_db_is_rekeyed_to_sqlcipher_on_startup() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_plaintext_state_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("state.sqlite");
    {
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute("CREATE TABLE plaintext_marker (id INTEGER PRIMARY KEY)", [])
            .unwrap();
    }

    let engine = PersistenceEngine::initialize_at(db_path.clone()).unwrap();
    let connection = engine.open_connection().unwrap();
    let _: i64 = connection
        .query_row("SELECT COUNT(*) FROM plaintext_marker", [], |row| {
            row.get(0)
        })
        .unwrap();
    drop(connection);
    drop(engine);

    let mut file = std::fs::File::open(&db_path).unwrap();
    let mut header = [0u8; 16];
    file.read_exact(&mut header).unwrap();
    assert_ne!(&header, b"SQLite format 3\0");

    let unkeyed = Connection::open(&db_path).unwrap();
    let unkeyed_read = unkeyed.query_row("SELECT COUNT(*) FROM plaintext_marker", [], |_| Ok(()));
    assert!(unkeyed_read.is_err());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn production_keychain_failure_refuses_database_initialization() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_keychain_fail_closed_{}", unix_time_ms()));
    let db_path = temp_dir.join("fail_closed.sqlite");

    let error = match PersistenceEngine::initialize_at_with_database_key_loader(
        db_path,
        || Err("mock production keychain unavailable".to_string()),
        false,
    ) {
        Ok(_) => panic!("database initialization must fail closed"),
        Err(error) => error,
    };

    assert!(
        error.contains("mock production keychain unavailable"),
        "unexpected error: {error}"
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sqlcipher_invalid_key_fails_closed() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_invalid_key_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("state.sqlite");
    let engine = PersistenceEngine::initialize_at(db_path.clone()).unwrap();
    engine
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('invalid-key-canary', 'secret', ?1, ?2)",
                params![unix_time_ms(), get_current_encryption_state()],
            )
            .unwrap();
    drop(engine);

    let error =
        open_sqlcipher_database_connection_with_key(&db_path, "definitely-the-wrong-database-key")
            .unwrap_err()
            .to_string();
    assert!(!error.is_empty());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn corrupt_sqlcipher_database_fails_closed() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_corrupt_db_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("state.sqlite");
    std::fs::write(
        &db_path,
        b"corrupt encrypted database bytes that are not SQLite",
    )
    .unwrap();
    let key = get_database_key().unwrap();

    let error = open_sqlcipher_database_connection_with_key(&db_path, &key)
        .unwrap_err()
        .to_string();
    assert!(!error.is_empty());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn historical_session_config_schema_keeps_its_signed_8192_default() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_session_config_default_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("session.sqlite")).unwrap();

    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO active_session_configs (session_id) VALUES (?1)",
            params!["session-default"],
        )
        .unwrap();

    let config = engine
        .select_session_config("session-default")
        .unwrap()
        .expect("session config should use schema defaults");
    assert_eq!(config.context_budget, 8_192);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn workflow_execution_migration_enforces_contracts() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(WORKFLOW_EXECUTION_MIGRATION)
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../../migrations/0003_workflow_compilation_status.sql"
        ))
        .unwrap();
    connection
        .execute_batch(WORKFLOW_APPROVAL_GATEWAY_MIGRATION)
        .unwrap();
    connection
        .execute_batch(WORKFLOW_SCHEDULES_MIGRATION)
        .unwrap();

    connection
        .execute(
            "
                INSERT INTO workflow_blueprints (
                    workflow_id, version, name, visual_state_json, workflow_ir_json,
                    is_active, created_at_ms, updated_at_ms
                ) VALUES ('wf-test', 1, 'Test', '{}', '{}', 1, 10, 10)
                ",
            [],
        )
        .unwrap();
    let duplicate_active = connection.execute(
        "
            INSERT INTO workflow_blueprints (
                workflow_id, version, name, visual_state_json, is_active,
                created_at_ms, updated_at_ms
            ) VALUES ('wf-test', 2, 'Test v2', '{}', 1, 11, 11)
            ",
        [],
    );
    assert!(duplicate_active.is_err());

    let invalid_compilation_status = connection.execute(
        "
            UPDATE workflow_blueprints
            SET compilation_status = 'Unknown'
            WHERE workflow_id = 'wf-test' AND version = 1
            ",
        [],
    );
    assert!(invalid_compilation_status.is_err());

    let invalid_status = connection.execute(
        "
            INSERT INTO execution_instances (
                id, workflow_id, workflow_version, status, created_at_ms, updated_at_ms
            ) VALUES ('run-test', 'wf-test', 1, 'Unknown', 10, 10)
            ",
        [],
    );
    assert!(invalid_status.is_err());

    connection
        .execute(
            "
                INSERT INTO workflow_schedules (
                    id, workflow_id, workflow_version, label, schedule_expression,
                    run_request_json, is_active, next_run_at_ms, created_at_ms, updated_at_ms
                ) VALUES (
                    'sched-test', 'wf-test', 1, 'Test', 'every 2 minutes',
                    '{}', 1, 120000, 10, 10
                )
                ",
            [],
        )
        .unwrap();
    let invalid_schedule_state = connection.execute(
        "
            UPDATE workflow_schedules
            SET last_status = 'Unknown'
            WHERE id = 'sched-test'
            ",
        [],
    );
    assert!(invalid_schedule_state.is_err());
}

#[test]
fn execution_transcript_continuity_repairs_terminal_turns_and_removes_internal_receipts() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_execution_transcript_continuity_{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-continuity".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Transcript continuity".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-continuity".to_string(),
        generation_token: "generation-continuity".to_string(),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        parent_turn_id: None,
        root_turn_id: "turn-continuity".to_string(),
        turn_kind: "root".to_string(),
    };
    engine
        .accept_chat_turn(AcceptChatTurnRequest {
            turn_id: context.turn_id.clone(),
            generation_token: context.generation_token.clone(),
            parent_turn_id: None,
            root_turn_id: context.root_turn_id.clone(),
            turn_kind: context.turn_kind.clone(),
            session_id: context.session_id.clone(),
            agent_id: context.agent_id.clone(),
            provider_id: context.provider_id.clone(),
            model_id: context.model_id.clone(),
            message: "Inspect this project.".to_string(),
        })
        .unwrap();
    engine.finish_chat_turn(&context, "completed").unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE chat_messages
             SET metadata_json=json_set(metadata_json,'$.turnState','accepted')
             WHERE session_id=?1 AND role='user'",
            params![session.id],
        )
        .unwrap();
    drop(connection);
    engine
        .insert_chat_message_with_metadata(
            &session.id,
            &session.agent_id,
            "assistant",
            "Logical Certificate Receipt\nInternal execution evidence.",
            Some(&session.provider_id),
            Some(&session.model_id),
            Some(&json!({"schema": "oomu.agent_execution_terminal.v1"})),
        )
        .unwrap();

    engine
        .open_connection()
        .unwrap()
        .execute_batch(static_migrations::EXECUTION_TRANSCRIPT_CONTINUITY_SQL)
        .unwrap();

    let messages = engine.select_chat_messages(&session.id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
    assert!(messages[0]
        .metadata_json
        .as_deref()
        .unwrap()
        .contains("\"turnState\":\"completed\""));
    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}
