use crate::foundation::digest::sha256_hex;
use argon2::{Algorithm, Argon2, Params, Version};
use std::{
    io,
    sync::{Mutex, OnceLock},
};
use zeroize::Zeroize;

#[cfg(not(test))]
const DATABASE_KEY_MEMORY_KIB: u32 = 19 * 1024;
#[cfg(not(test))]
const DATABASE_KEY_ITERATIONS: u32 = 3;
const DATABASE_KEY_PARALLELISM: u32 = 1;
#[cfg(test)]
const INTEGRATION_TEST_KEY_MEMORY_KIB: u32 = 64;
#[cfg(test)]
const INTEGRATION_TEST_KEY_ITERATIONS: u32 = 1;

static CACHED_DB_KEY: OnceLock<Mutex<CachedDatabaseKey>> = OnceLock::new();

enum CachedDatabaseKey {
    Empty,
    Ready(DatabaseKeyMaterial),
    Failed(String),
}

struct DatabaseKeyMaterial {
    key: String,
}

impl DatabaseKeyMaterial {
    fn new(key: String) -> Self {
        Self { key }
    }

    fn expose(&self) -> String {
        self.key.clone()
    }
}

impl Drop for DatabaseKeyMaterial {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

pub fn get_database_key() -> Result<String, String> {
    let cache = CACHED_DB_KEY.get_or_init(|| Mutex::new(CachedDatabaseKey::Empty));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match &*cache {
        CachedDatabaseKey::Ready(material) => Ok(material.expose()),
        CachedDatabaseKey::Failed(message) => Err(message.clone()),
        CachedDatabaseKey::Empty => match resolve_database_key() {
            Ok(key) => {
                *cache = CachedDatabaseKey::Ready(DatabaseKeyMaterial::new(key));
                match &*cache {
                    CachedDatabaseKey::Ready(material) => Ok(material.expose()),
                    _ => Err("Database key cache failed to initialize.".to_string()),
                }
            }
            Err(message) => {
                *cache = CachedDatabaseKey::Failed(message.clone());
                Err(message)
            }
        },
    }
}

pub(super) fn clear_cached_database_key() {
    if let Some(cache) = CACHED_DB_KEY.get() {
        let mut cache = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cache = CachedDatabaseKey::Empty;
    }
}

pub fn database_key_error(message: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(io::Error::new(io::ErrorKind::Other, message)))
}

fn resolve_database_key() -> Result<String, String> {
    let mut database_secret = resolve_database_secret_for_key_derivation()?;
    let database_key = derive_memory_hard_database_key(&database_secret);
    database_secret.zeroize();
    database_key
}

#[cfg(test)]
fn resolve_database_secret_for_key_derivation() -> Result<String, String> {
    if let Some(secret) = crate::launch_startup::sprint_294_isolated_profile::database_secret() {
        return Ok(secret.as_str().to_string());
    }
    test_database_secret_fallback(true)
        .ok_or_else(|| "Test database key fallback is unavailable.".to_string())
}

#[cfg(not(test))]
fn resolve_database_secret_for_key_derivation() -> Result<String, String> {
    if let Some(secret) = crate::launch_startup::sprint_294_isolated_profile::database_secret() {
        return Ok(secret.as_str().to_string());
    }
    if let Some(secret) = crate::scenario_one_e2e_profile::database_secret() {
        return Ok(secret.as_str().to_string());
    }
    resolve_database_secret(load_keychain_database_secret, cfg!(test)).map_err(|message| {
        eprintln!(
            "CRITICAL_DATABASE_KEY_UNAVAILABLE code=database_keyring_unavailable boundary=SQLCipher"
        );
        message
    })
}

pub(super) fn get_legacy_database_key_for_migration() -> Result<String, String> {
    let mut database_secret = resolve_database_secret_for_key_derivation()?;
    let mut legacy_key = derive_legacy_bound_database_key(&database_secret);
    database_secret.zeroize();
    if legacy_key.is_empty() {
        legacy_key.zeroize();
        return Err("Legacy SQLCipher migration key derivation failed.".to_string());
    }
    Ok(legacy_key)
}

#[cfg(not(test))]
fn resolve_database_secret<F>(
    keychain_loader: F,
    allow_insecure_test_fallback: bool,
) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String>,
{
    resolve_database_secret_with_keychain_mode(keychain_loader, allow_insecure_test_fallback)
}

#[cfg(any(test, debug_assertions))]
pub(super) fn resolve_database_secret_with_keychain_mode<F>(
    keychain_loader: F,
    allow_insecure_test_fallback: bool,
) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String>,
{
    resolve_database_secret_with_keychain_mode_inner(keychain_loader, allow_insecure_test_fallback)
}

#[cfg(not(any(test, debug_assertions)))]
fn resolve_database_secret_with_keychain_mode<F>(
    keychain_loader: F,
    allow_insecure_test_fallback: bool,
) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String>,
{
    resolve_database_secret_with_keychain_mode_inner(keychain_loader, allow_insecure_test_fallback)
}

fn resolve_database_secret_with_keychain_mode_inner<F>(
    keychain_loader: F,
    allow_insecure_test_fallback: bool,
) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String>,
{
    keychain_loader().or_else(|error| {
        test_database_secret_fallback(allow_insecure_test_fallback).ok_or_else(|| {
            format!(
                "Database keyring is unavailable and no plaintext database-key fallback is permitted: {error}"
            )
        })
    })
}

#[cfg(not(test))]
fn load_keychain_database_secret() -> Result<String, String> {
    let (service, account) = crate::keychain_namespace::sovereign_identity_location();
    crate::keychain_session::get_password(service, account)
        .map_err(|error| format!("Database keychain secret is unavailable: {error}"))?
        .ok_or_else(|| "Database keychain secret is unavailable.".to_string())
}

fn test_database_secret_fallback(allow: bool) -> Option<String> {
    if !allow {
        return None;
    }
    #[cfg(test)]
    {
        Some("default_secure_test_key".to_string())
    }
    #[cfg(not(test))]
    {
        None
    }
}

#[cfg(test)]
pub(super) fn derive_memory_hard_database_key(database_secret: &str) -> Result<String, String> {
    derive_integration_test_database_key(database_secret)
}

#[cfg(all(not(test), debug_assertions))]
pub(super) fn derive_memory_hard_database_key(database_secret: &str) -> Result<String, String> {
    derive_memory_hard_database_key_inner(database_secret)
}

#[cfg(not(any(test, debug_assertions)))]
fn derive_memory_hard_database_key(database_secret: &str) -> Result<String, String> {
    derive_memory_hard_database_key_inner(database_secret)
}

#[cfg(not(test))]
fn derive_memory_hard_database_key_inner(database_secret: &str) -> Result<String, String> {
    derive_database_key_with_params(
        database_secret,
        DATABASE_KEY_MEMORY_KIB,
        DATABASE_KEY_ITERATIONS,
    )
}

#[cfg(test)]
pub(super) fn derive_integration_test_database_key(
    database_secret: &str,
) -> Result<String, String> {
    derive_database_key_with_params(
        database_secret,
        INTEGRATION_TEST_KEY_MEMORY_KIB,
        INTEGRATION_TEST_KEY_ITERATIONS,
    )
}

fn derive_database_key_with_params(
    database_secret: &str,
    memory_kib: u32,
    iterations: u32,
) -> Result<String, String> {
    let mut key = [0_u8; 32];
    let salt = format!(
        "oomu-sqlcipher-database-key-v1:{}:{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let params = Params::new(
        memory_kib,
        iterations,
        DATABASE_KEY_PARALLELISM,
        Some(key.len()),
    )
    .map_err(|error| format!("Invalid Argon2id database key parameters: {error}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    if let Err(error) =
        argon2.hash_password_into(database_secret.as_bytes(), salt.as_bytes(), &mut key)
    {
        key.zeroize();
        return Err(format!("Argon2id database key derivation failed: {error}"));
    }
    let encoded = hex::encode(key);
    key.zeroize();
    Ok(encoded)
}

#[cfg(test)]
pub(super) fn derive_legacy_bound_database_key(database_secret: &str) -> String {
    derive_legacy_bound_database_key_inner(database_secret)
}

#[cfg(not(test))]
fn derive_legacy_bound_database_key(database_secret: &str) -> String {
    derive_legacy_bound_database_key_inner(database_secret)
}

fn derive_legacy_bound_database_key_inner(database_secret: &str) -> String {
    let hw_binding =
        sha256_hex(format!("{}:{}", std::env::consts::OS, std::env::consts::ARCH).as_bytes());
    sha256_hex(format!("{}:{}", database_secret, hw_binding).as_bytes())
}

#[cfg(test)]
pub fn get_current_encryption_state() -> &'static str {
    "test_argon2id_aes256"
}

#[cfg(not(test))]
pub fn get_current_encryption_state() -> &'static str {
    "hardware_locked_argon2id_aes256"
}
