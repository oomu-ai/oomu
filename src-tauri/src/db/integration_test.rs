use super::{
    database_key::get_database_key, default_workspace_id, BackingStoreClass, PersistenceEngine,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};
#[cfg(test)]
use zeroize::Zeroize;

#[cfg(test)]
use super::database_key::{
    derive_memory_hard_database_key, resolve_database_secret_with_keychain_mode,
};

#[cfg(test)]
pub(super) fn test_ops_path(db_path: &Path) -> Option<PathBuf> {
    db_path
        .parent()
        .filter(|parent| *parent == std::env::temp_dir())
        .map(|_| db_path.with_extension("ops.db"))
}

#[cfg(not(test))]
pub(super) fn test_ops_path(_db_path: &Path) -> Option<PathBuf> {
    None
}

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
        let database_key = get_database_key()?;
        Self::initialize_at_with_database_key(db_path, &database_key)
    }
}
