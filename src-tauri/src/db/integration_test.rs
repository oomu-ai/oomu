#[cfg(debug_assertions)]
use super::database_key::derive_integration_test_database_key;
#[cfg(test)]
use super::database_key::{
    derive_memory_hard_database_key, resolve_database_secret_with_keychain_mode,
};
use super::{database_key::get_database_key, PersistenceEngine};
#[cfg(any(test, debug_assertions))]
use super::{default_workspace_id, BackingStoreClass};
#[cfg(debug_assertions)]
use std::{collections::HashMap, sync::OnceLock};
#[cfg(any(test, debug_assertions))]
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};
#[cfg(test)]
use zeroize::Zeroize;

#[cfg(debug_assertions)]
static INTEGRATION_TEST_DATABASE_KEYS: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();

#[cfg(debug_assertions)]
pub(super) fn key(engine: &PersistenceEngine) -> Result<String, String> {
    let path = engine
        .db_path
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(key) = INTEGRATION_TEST_DATABASE_KEYS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(path.as_path())
    {
        return Ok(key.clone());
    }
    get_database_key()
}

#[cfg(not(debug_assertions))]
pub(super) fn key(_engine: &PersistenceEngine) -> Result<String, String> {
    get_database_key()
}

#[cfg(test)]
pub(super) fn test_ops_path(db_path: &Path) -> Option<PathBuf> {
    db_path
        .parent()
        .filter(|parent| *parent == std::env::temp_dir())
        .map(|_| db_path.with_extension("ops.db"))
}

#[cfg(all(not(test), debug_assertions))]
pub(super) fn test_ops_path(_db_path: &Path) -> Option<PathBuf> {
    None
}

#[cfg(any(test, debug_assertions))]
impl PersistenceEngine {
    #[cfg(test)]
    pub(super) fn initialize_at_with_database_key_loader<F>(
        db_path: PathBuf,
        keychain_loader: F,
        allow_insecure_test_fallback: bool,
    ) -> Result<Self, String>
    where
        F: FnOnce() -> Result<String, String>,
    {
        let mut database_secret = resolve_database_secret_with_keychain_mode(
            keychain_loader,
            allow_insecure_test_fallback,
        )?;
        let database_key = derive_memory_hard_database_key(&database_secret)?;
        database_secret.zeroize();
        Self::initialize_at_with_database_key(db_path, &database_key)
    }

    fn initialize_at_with_database_key(
        db_path: PathBuf,
        database_key: &str,
    ) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let operations_path = db_path.with_extension("ops.db");
        #[cfg(debug_assertions)]
        INTEGRATION_TEST_DATABASE_KEYS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(db_path.clone(), database_key.to_string());
        let engine = Self {
            db_path: Arc::new(RwLock::new(db_path)),
            write_lock: Arc::new(Mutex::new(())),
            workspace_id: default_workspace_id(),
            storage_class: Arc::new(RwLock::new(BackingStoreClass::Persistent)),
            ops_path: Some(operations_path),
        };
        engine
            .run_migrations_with_database_key(database_key)
            .map_err(|error| error.to_string())?;
        Ok(engine)
    }

    /// Creates an isolated encrypted store for integration tests without
    /// depending on the interactive OS keychain. This API is absent from
    /// optimized release builds.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn initialize_for_integration_test(db_path: PathBuf) -> Result<Self, String> {
        let database_key = derive_integration_test_database_key(
            "oomu-isolated-integration-test-database-secret-v1",
        )?;
        Self::initialize_at_with_database_key(db_path, &database_key)
    }
}
