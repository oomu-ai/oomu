use super::*;

#[test]
fn volatile_records_reconcile_to_durable_store_and_survive_restart() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_reconcile_db_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let durable_path = temp_dir.join("durable.sqlite");
    let volatile_path = temp_dir.join("volatile.sqlite");
    let durable = PersistenceEngine::initialize_at(durable_path.clone()).unwrap();
    durable
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('durable-record', 'original', ?1, ?2)",
            params![unix_time_ms(), get_current_encryption_state()],
        )
        .unwrap();
    drop(durable);
    let volatile = PersistenceEngine::initialize_volatile_at(volatile_path).unwrap();
    volatile
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('volatile-record', 'recover-me', ?1, ?2)",
            params![unix_time_ms(), get_current_encryption_state()],
        )
        .unwrap();

    let backup_dir = temp_dir.join(".oomu-migration-backups");
    let backups_before = std::fs::read_dir(&backup_dir).unwrap().count();
    let recovered = volatile
        .reconcile_volatile_store_to(durable_path.clone(), false)
        .unwrap();
    assert!(!recovered.requires_confirmation);
    assert!(recovered.recovered_records > 0);
    assert_eq!(recovered.conflicting_records, 0);
    assert!(recovered.backup_created);
    assert!(std::fs::read_dir(&backup_dir).unwrap().count() > backups_before);
    assert_eq!(volatile.storage_class(), BackingStoreClass::Persistent);
    assert_eq!(volatile.db_path(), durable_path.to_string_lossy());

    let restarted = PersistenceEngine::initialize_at(durable_path).unwrap();
    let recovered_value: String = restarted
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT value FROM app_preferences WHERE key='volatile-record'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recovered_value, "recover-me");
    let preserved_value: String = restarted
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT value FROM app_preferences WHERE key='durable-record'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(preserved_value, "original");
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn volatile_recovery_reconciles_into_a_verified_early_beta_schema() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_beta_reconcile_db_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let durable_path = temp_dir.join("durable.sqlite");
    let volatile_path = temp_dir.join("volatile.sqlite");
    let durable = PersistenceEngine::initialize_at(durable_path.clone()).unwrap();
    durable
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE schema_migration_ledger SET checksum_sha256=?1 WHERE sequence=23",
            params!["c".repeat(64)],
        )
        .unwrap();
    drop(durable);

    let volatile = PersistenceEngine::initialize_volatile_at(volatile_path).unwrap();
    volatile
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state) VALUES ('beta-recovery-record', 'recover-me', ?1, ?2)",
            params![unix_time_ms(), get_current_encryption_state()],
        )
        .unwrap();

    let report = volatile
        .reconcile_volatile_store_to(durable_path.clone(), false)
        .unwrap();
    assert!(!report.requires_confirmation);
    assert!(report.durable_probe_verified);
    assert!(report.recovered_records > 0);

    let restarted = PersistenceEngine::initialize_at(durable_path).unwrap();
    let recovered_value: String = restarted
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT value FROM app_preferences WHERE key='beta-recovery-record'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recovered_value, "recover-me");
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn volatile_reconciliation_requires_confirmation_for_a_genuine_value_conflict() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_reconcile_conflict_{}", unix_time_ms()));
    let durable_path = temp_dir.join("durable.sqlite");
    let volatile_path = temp_dir.join("volatile.sqlite");
    std::fs::create_dir_all(&temp_dir).unwrap();
    let durable = PersistenceEngine::initialize_at(durable_path.clone()).unwrap();
    durable
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO intents (id, plan_id, prompt, metadata, timestamp_ms, encryption_state) VALUES (1, 'durable-plan', 'durable', '{}', 2, ?1)",
            params![get_current_encryption_state()],
        )
        .unwrap();
    drop(durable);
    let volatile = PersistenceEngine::initialize_volatile_at(volatile_path).unwrap();
    volatile
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO intents (id, plan_id, prompt, metadata, timestamp_ms, encryption_state) VALUES (1, 'recovery-plan', 'recovery', '{}', 1, ?1)",
            params![get_current_encryption_state()],
        )
        .unwrap();

    let report = volatile
        .reconcile_volatile_store_to(durable_path, false)
        .unwrap();

    assert!(report.requires_confirmation);
    assert_eq!(report.conflicting_records, 1);
    assert_eq!(volatile.storage_class(), BackingStoreClass::RecoveryPending);
    let _ = std::fs::remove_dir_all(temp_dir);
}
