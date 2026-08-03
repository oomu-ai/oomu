use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_MANAGER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temporary_manager() -> AgentManager {
    let db_path = std::env::temp_dir().join(format!(
        "oomu-provider-metadata-no-keychain-{}-{}-{}.db",
        std::process::id(),
        unix_time_ms(),
        TEMP_MANAGER_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    ));
    let manager = AgentManager {
        db_path: Arc::new(db_path),
        write_lock: Arc::new(Mutex::new(())),
    };
    manager
        .run_migrations()
        .expect("prepare temporary provider database");
    manager
}

#[test]
fn provider_metadata_queries_do_not_read_keychain_secrets() {
    let manager = temporary_manager();
    let provider_id = "provider-metadata-no-keychain-id";
    let mut provider = ConfiguredProvider {
        id: provider_id.to_string(),
        provider_id: "google".to_string(),
        provider_name: "google".to_string(),
        auth_method: "api_key".to_string(),
        base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
        api_key_label: "TEST_API_KEY".to_string(),
        api_key: Some("provider-metadata-secret".to_string()),
        credential_configured: false,
        custom_model_ids: "gemini-3.5-flash".to_string(),
        auto_route_target: true,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    manager.upsert_provider_config(provider.clone()).unwrap();
    provider.api_key = None;
    crate::secret_store::evict_provider_secret_for_test(provider_id);
    let reads_before = crate::secret_store::provider_secret_backend_reads_for_test(provider_id);

    let listed = manager.select_provider_configs().unwrap();
    let active = manager.get_active_auto_route_target().unwrap().unwrap();

    assert!(listed.iter().any(|item| {
        item.id == provider_id && item.credential_configured && item.api_key.is_none()
    }));
    assert_eq!(active.id, provider_id);
    assert!(active.credential_configured);
    assert!(active.api_key.is_none());
    assert_eq!(
        crate::secret_store::provider_secret_backend_reads_for_test(provider_id),
        reads_before
    );
    let _ = std::fs::remove_file(manager.db_path.as_ref());
}

#[test]
fn provider_metadata_recovers_when_keychain_item_was_removed_externally() {
    let manager = temporary_manager();
    let provider_id = "provider-metadata-missing-keychain-item";
    let mut provider = ConfiguredProvider {
        id: provider_id.to_string(),
        provider_id: "google".to_string(),
        provider_name: "Google".to_string(),
        auth_method: "api_key".to_string(),
        base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
        api_key_label: "GOOGLE_API_KEY".to_string(),
        api_key: Some("removed-outside-oomu".to_string()),
        credential_configured: false,
        custom_model_ids: "gemini-3.5-flash".to_string(),
        auto_route_target: true,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    manager.upsert_provider_config(provider.clone()).unwrap();
    provider.api_key = None;
    crate::secret_store::remove_provider_secret_backend_value_for_test(provider_id);
    let reads_before = crate::secret_store::provider_secret_backend_reads_for_test(provider_id);

    let listed = manager.select_provider_configs().unwrap();
    let active = manager.get_active_auto_route_target().unwrap().unwrap();

    assert!(listed
        .iter()
        .any(|item| item.id == provider_id && !item.credential_configured));
    assert!(!active.credential_configured);
    assert_eq!(
        crate::secret_store::provider_secret_backend_reads_for_test(provider_id),
        reads_before
    );
    let persisted: i64 = manager
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT credential_configured FROM provider_configs WHERE id = ?1",
            params![provider_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted, 0);
    let _ = std::fs::remove_file(manager.db_path.as_ref());
}

#[test]
fn legacy_provider_secret_migrates_only_when_that_provider_is_used() {
    let manager = temporary_manager();
    let provider_id = "provider-lazy-keychain-migration";
    let connection = manager.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO provider_configs (id, provider_id, provider_name, auth_method, base_url, api_key_label, api_key, credential_configured, custom_model_ids, auto_route_target, created_at_ms, updated_at_ms) VALUES (?1, 'google', 'Google', 'api_key', 'https://generativelanguage.googleapis.com/v1beta', 'GOOGLE_API_KEY', 'legacy-secret', 0, 'gemini-3.5-flash', 1, 1, 1)",
            params![provider_id],
        )
        .unwrap();
    drop(connection);
    crate::secret_store::evict_provider_secret_for_test(provider_id);
    let reads_before = crate::secret_store::provider_secret_backend_reads_for_test(provider_id);

    let listed = manager.select_provider_configs().unwrap();
    assert!(listed
        .iter()
        .any(|item| item.id == provider_id && item.credential_configured));
    assert_eq!(
        crate::secret_store::provider_secret_backend_reads_for_test(provider_id),
        reads_before
    );

    let selected = manager
        .select_provider_config(provider_id)
        .unwrap()
        .unwrap();
    assert_eq!(selected.api_key.as_deref(), Some("legacy-secret"));
    let connection = manager.open_connection().unwrap();
    let legacy: Option<String> = connection
        .query_row(
            "SELECT api_key FROM provider_configs WHERE id = ?1",
            params![provider_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(legacy.is_none());
    let _ = std::fs::remove_file(manager.db_path.as_ref());
}
