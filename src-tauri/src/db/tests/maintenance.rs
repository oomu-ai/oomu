use super::*;

#[test]
fn encrypted_recovery_snapshot_waits_for_writer_and_exports_one_consistent_state() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_snapshot_export_{}", unix_time_ms()));
    let bundle_dir = temp_dir.join("bundle");
    std::fs::create_dir_all(&bundle_dir).unwrap();
    let source_path = temp_dir.join("volatile.sqlite");
    let snapshot_path = bundle_dir.join("state.sqlite");
    let operations_snapshot_path = bundle_dir.join(OPS_DB_FILE);
    let engine = PersistenceEngine::initialize_volatile_at(source_path.clone()).unwrap();

    let (writer_ready_tx, writer_ready_rx) = std::sync::mpsc::channel();
    let (commit_tx, commit_rx) = std::sync::mpsc::channel();
    let writer_engine = engine.clone();
    let writer = std::thread::spawn(move || {
        let _guard = writer_engine.lock_writes();
        let mut connection = writer_engine.open_connection().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
                .execute(
                    "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('snapshot-canary', 'committed', ?1, ?2)",
                    params![unix_time_ms(), get_current_encryption_state()],
                )
                .unwrap();
        writer_ready_tx.send(()).unwrap();
        commit_rx.recv().unwrap();
        transaction.commit().unwrap();
    });
    writer_ready_rx.recv().unwrap();

    let (export_done_tx, export_done_rx) = std::sync::mpsc::channel();
    let export_engine = engine.clone();
    let export_source = source_path.clone();
    let export_snapshot = snapshot_path.clone();
    let exporter = std::thread::spawn(move || {
        export_done_tx
            .send(export_engine.export_encrypted_snapshot(&export_source, &export_snapshot))
            .unwrap();
    });
    assert!(export_done_rx
        .recv_timeout(Duration::from_millis(100))
        .is_err());
    commit_tx.send(()).unwrap();
    writer.join().unwrap();
    export_done_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    exporter.join().unwrap();

    let operations_source_path = temp_dir.join(OPS_DB_FILE);
    engine
        .export_encrypted_operations_snapshot(&operations_source_path, &operations_snapshot_path)
        .unwrap();

    assert!(snapshot_path.is_file());
    assert!(!has_plaintext_sqlite_header(&snapshot_path));
    assert!(operations_snapshot_path.is_file());
    assert!(!has_plaintext_sqlite_header(&operations_snapshot_path));
    assert!(!snapshot_path.with_extension("sqlite-wal").exists());
    assert!(!snapshot_path.with_extension("sqlite-shm").exists());
    let key = get_database_key().unwrap();
    let snapshot = open_sqlcipher_database_connection_with_key(&snapshot_path, &key).unwrap();
    verify_migration_ledger(&snapshot).unwrap();
    verify_schema_invariants(&snapshot, MIGRATIONS.len() as i64).unwrap();
    let canary: String = snapshot
        .query_row(
            "SELECT value FROM app_preferences WHERE key='snapshot-canary'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(canary, "committed");
    let operations_snapshot =
        open_sqlcipher_database_connection_with_key(&operations_snapshot_path, &key).unwrap();
    verify_operations_database(&operations_snapshot).unwrap();
    drop(snapshot);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn reset_state_purges_transient_runtime_tables() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_reset_state_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("reset.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "
                INSERT INTO message_queue (
                    agent_id, message, attachments_json, status, created_at_ms, updated_at_ms
                )
                VALUES ('agent-test', 'queued', '[]', 'queued', ?1, ?1)
                ",
            params![unix_time_ms()],
        )
        .unwrap();
    connection
        .execute(
            "
                INSERT INTO agent_execution_logs (
                    execution_id, plan_id, level, phase, message, created_at_ms
                )
                VALUES ('exec-1', 'plan-1', 'info', 'running', 'hello', ?1)
                ",
            params![unix_time_ms()],
        )
        .unwrap();
    connection
        .execute(
            "
                INSERT INTO plan_generation_states (
                    plan_id, plan_json, current_step_index, status, generated_text, timestamp_ms
                )
                VALUES ('plan-1', '{}', 0, 'running', '', ?1)
                ",
            params![unix_time_ms()],
        )
        .unwrap();
    connection
        .execute(
            "
                INSERT INTO actions (plan_id, tool, input, status, timestamp_ms)
                VALUES ('plan-1', 'test', '{}', 'running', ?1)
                ",
            params![unix_time_ms()],
        )
        .unwrap();
    connection
        .execute_batch(
            "
                CREATE TABLE temp_runtime_cache (id INTEGER PRIMARY KEY);
                INSERT INTO temp_runtime_cache (id) VALUES (1);
                ",
        )
        .unwrap();

    purge_transient_sqlite_cache_on_connection(&connection).unwrap();

    let queued_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM message_queue", [], |row| row.get(0))
        .unwrap();
    let log_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_execution_logs", [], |row| {
            row.get(0)
        })
        .unwrap();
    let plan_state_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM plan_generation_states", [], |row| {
            row.get(0)
        })
        .unwrap();
    let action_status: String = connection
        .query_row(
            "SELECT status FROM actions WHERE plan_id='plan-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(queued_count, 0);
    assert_eq!(log_count, 0);
    assert_eq!(plan_state_count, 0);
    assert_eq!(action_status, "recoverable");
    assert!(!table_exists(&connection, "temp_runtime_cache").unwrap());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn release_database_sanitizer_purges_distribution_state_tables() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_release_sanitize_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("release.sqlite");
    let engine = PersistenceEngine::initialize_at(db_path.clone()).unwrap();
    let connection = engine.open_connection().unwrap();
    let now = unix_time_ms();

    connection
            .execute(
                "
                INSERT INTO chat_sessions (
                    id, workspace_id, agent_id, title, provider_id, model_id, created_at_ms, updated_at_ms
                )
                VALUES ('session-1', '{}', 'agent-test', 'Release test', 'local', 'model', ?1, ?1)
                ",
                params![now],
            )
            .unwrap();
    connection
        .execute(
            "
                INSERT INTO chat_messages (
                    workspace_id, session_id, agent_id, role, content, timestamp_ms
                )
                VALUES ('{}', 'session-1', 'agent-test', 'user', 'hello', ?1)
                ",
            params![now],
        )
        .unwrap();
    connection
        .execute(
            "
                INSERT INTO intents (plan_id, prompt, metadata, timestamp_ms)
                VALUES ('plan-1', 'prompt', '{}', ?1)
                ",
            params![now],
        )
        .unwrap();
    connection
        .execute(
            "
                INSERT INTO actions (plan_id, tool, input, status, timestamp_ms)
                VALUES ('plan-1', 'test', '{}', 'complete', ?1)
                ",
            params![now],
        )
        .unwrap();
    connection
        .execute(
            "
                INSERT INTO certificates (plan_id, mlc_path, mlc_content, timestamp_ms)
                VALUES ('plan-1', '/tmp/cert.mlc', 'certificate', ?1)
                ",
            params![now],
        )
        .unwrap();
    connection
        .execute(
            "
                INSERT INTO plan_generation_states (
                    plan_id, plan_json, current_step_index, status, generated_text, timestamp_ms
                )
                VALUES ('plan-1', '{}', 0, 'complete', 'done', ?1)
                ",
            params![now],
        )
        .unwrap();
    connection
        .execute(
            "
                INSERT INTO agent_execution_logs (
                    execution_id, plan_id, level, phase, message, created_at_ms
                )
                VALUES ('exec-1', 'plan-1', 'info', 'complete', 'done', ?1)
                ",
            params![now],
        )
        .unwrap();
    drop(connection);
    drop(engine);

    let report = sanitize_release_database_at(&db_path).unwrap();
    assert_eq!(report.purged_tables.len(), 7);

    let connection = open_state_database_connection(&db_path).unwrap();
    for table in [
        "chat_messages",
        "chat_sessions",
        "agent_execution_logs",
        "plan_generation_states",
        "intents",
        "actions",
        "certificates",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} should be empty after release sanitation");
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sqlite_auto_vacuum_and_maintenance_are_incremental() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_sqlite_maintenance_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("maintenance.sqlite")).unwrap();

    let connection = engine.open_connection().unwrap();
    let auto_vacuum_mode: i64 = connection
        .query_row("PRAGMA auto_vacuum;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(auto_vacuum_mode, 2);
    connection
        .execute(
            "
                INSERT INTO routing_preferences (key, value, updated_at)
                VALUES ('maintenance-test', 'enabled', 1)
                ",
            [],
        )
        .unwrap();
    drop(connection);

    let now_ms = unix_time_ms();
    assert!(engine.run_sqlite_maintenance_if_due(now_ms).unwrap());
    let connection = engine.open_connection().unwrap();
    let stat_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM sqlite_stat1", [], |row| row.get(0))
        .unwrap();
    assert!(stat_count > 0);
    drop(connection);

    assert!(!engine
        .run_sqlite_maintenance_if_due(now_ms + 1_000)
        .unwrap());
    assert!(engine
        .run_sqlite_maintenance_if_due(now_ms + SQLITE_MAINTENANCE_INTERVAL_MS)
        .unwrap());

    let _ = std::fs::remove_dir_all(temp_dir);
}
