use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};
use zeroize::Zeroizing;

mod qualification_grant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStatus {
    Unverified,
    Available,
    Unavailable,
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct CacheKey {
    service: String,
    account: String,
}

enum CachedSecret {
    Present(Zeroizing<String>),
    Missing,
    Unavailable,
}

struct BackendAccessError {
    message: String,
    suppress_session: bool,
}

struct SessionCache {
    values: HashMap<CacheKey, CachedSecret>,
    status: SessionStatus,
    backend_suppressed: bool,
    #[cfg(test)]
    suppressed_test_thread: Option<std::thread::ThreadId>,
}

impl Default for SessionCache {
    fn default() -> Self {
        Self {
            values: HashMap::new(),
            status: SessionStatus::Unverified,
            backend_suppressed: false,
            #[cfg(test)]
            suppressed_test_thread: None,
        }
    }
}

fn session_cache() -> &'static Mutex<SessionCache> {
    static CACHE: OnceLock<Mutex<SessionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SessionCache::default()))
}

fn cache_key(service: &str, account: &str) -> CacheKey {
    CacheKey {
        service: service.to_string(),
        account: account.to_string(),
    }
}

fn backend_is_suppressed(cache: &SessionCache) -> bool {
    if !cache.backend_suppressed {
        return false;
    }
    #[cfg(test)]
    {
        cache.suppressed_test_thread == Some(std::thread::current().id())
    }
    #[cfg(not(test))]
    {
        true
    }
}

fn suppress_backend(cache: &mut SessionCache) {
    cache.backend_suppressed = true;
    #[cfg(test)]
    {
        cache.suppressed_test_thread = Some(std::thread::current().id());
    }
}

fn clear_backend_suppression(cache: &mut SessionCache) {
    cache.backend_suppressed = false;
    #[cfg(test)]
    {
        cache.suppressed_test_thread = None;
    }
}

fn authorize_keychain_backend(
    service: &str,
    account: &str,
    operation: qualification_grant::Operation,
) -> Result<(), String> {
    if crate::launch_startup::sprint_294_isolated_profile::is_active() {
        return qualification_grant::authorize(service, account, operation);
    }
    if crate::scenario_one_e2e_profile::enabled() {
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_TRACE stage=keychain status=blocked service={service} account={account}"
        );
        return Err("keychain_disabled_in_isolated_scenario_profile".to_string());
    }
    Ok(())
}

/// Returns a credential from a process-lifetime, zeroizing cache. The cache
/// lock intentionally remains held across the first backend access so two
/// concurrent callers cannot produce a macOS Keychain prompt storm.
pub(crate) fn get_password(service: &str, account: &str) -> Result<Option<String>, String> {
    authorize_keychain_backend(service, account, qualification_grant::Operation::Get)?;
    let key = cache_key(service, account);
    let mut cache = session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.values.get(&key) {
        return Ok(match cached {
            CachedSecret::Present(secret) => Some(secret.as_str().to_string()),
            CachedSecret::Missing => None,
            CachedSecret::Unavailable => {
                return Err("keychain_session_access_unavailable".to_string())
            }
        });
    }
    // A denied or otherwise unavailable Keychain backend is process-wide on
    // macOS, not specific to one service/account pair. Once the user cancels
    // one prompt, fail every uncached lookup closed for the rest of the
    // session so independent credential consumers cannot queue a prompt
    // storm. An explicit recovery action can reopen the backend through
    // `retry_password` below.
    if backend_is_suppressed(&cache) {
        return Err("keychain_session_access_unavailable".to_string());
    }

    match backend_get_password(service, account) {
        Ok(value) => {
            cache.status = SessionStatus::Available;
            cache.values.insert(
                key,
                match value.as_deref() {
                    Some(secret) => CachedSecret::Present(Zeroizing::new(secret.to_string())),
                    None => CachedSecret::Missing,
                },
            );
            Ok(value)
        }
        Err(error) => {
            cache.status = SessionStatus::Unavailable;
            if error.suppress_session {
                suppress_backend(&mut cache);
            }
            cache.values.insert(key, CachedSecret::Unavailable);
            Err(error.message)
        }
    }
}

/// Checks whether a credential still exists without loading its secret. On
/// macOS this uses an attribute-only Keychain query with authentication UI
/// disabled, so provider status refreshes cannot produce password prompts.
pub(crate) fn password_exists(service: &str, account: &str) -> Result<bool, String> {
    authorize_keychain_backend(service, account, qualification_grant::Operation::Exists)?;
    let _cache = session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    backend_password_exists(service, account).map_err(|error| error.message)
}

/// Performs one deliberate backend retry after an unavailable or missing
/// result was cached. Ordinary callers continue using `get_password`, so a
/// failed Keychain prompt cannot turn into a prompt loop.
pub(crate) fn retry_password(service: &str, account: &str) -> Result<Option<String>, String> {
    authorize_keychain_backend(service, account, qualification_grant::Operation::Retry)?;
    let key = cache_key(service, account);
    {
        let mut cache = session_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match cache.values.get(&key) {
            Some(CachedSecret::Present(secret)) => return Ok(Some(secret.as_str().to_string())),
            Some(CachedSecret::Missing) | Some(CachedSecret::Unavailable) => {}
            None => {}
        }
        cache.values.remove(&key);
        cache.status = SessionStatus::Unverified;
        clear_backend_suppression(&mut cache);
    }
    get_password(service, account)
}

pub(crate) fn set_password(service: &str, account: &str, secret: &str) -> Result<(), String> {
    authorize_keychain_backend(service, account, qualification_grant::Operation::Set)?;
    let key = cache_key(service, account);
    let mut cache = session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if backend_is_suppressed(&cache) {
        return Err("keychain_session_access_unavailable".to_string());
    }
    match backend_set_password(service, account, secret) {
        Ok(()) => {
            cache.status = SessionStatus::Available;
            cache.values.insert(
                key,
                CachedSecret::Present(Zeroizing::new(secret.to_string())),
            );
            Ok(())
        }
        Err(error) => {
            cache.status = SessionStatus::Unavailable;
            if error.suppress_session {
                suppress_backend(&mut cache);
            }
            cache.values.insert(key, CachedSecret::Unavailable);
            Err(error.message)
        }
    }
}

pub(crate) fn delete_password(service: &str, account: &str) -> Result<(), String> {
    authorize_keychain_backend(service, account, qualification_grant::Operation::Delete)?;
    let key = cache_key(service, account);
    let mut cache = session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if backend_is_suppressed(&cache) {
        return Err("keychain_session_access_unavailable".to_string());
    }
    match backend_delete_password(service, account) {
        Ok(()) => {
            cache.status = SessionStatus::Available;
            cache.values.insert(key, CachedSecret::Missing);
            Ok(())
        }
        Err(error) => {
            cache.status = SessionStatus::Unavailable;
            if error.suppress_session {
                suppress_backend(&mut cache);
            }
            cache.values.insert(key, CachedSecret::Unavailable);
            Err(error.message)
        }
    }
}

pub(crate) fn status() -> SessionStatus {
    session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .status
}

/// Drops and zeroizes all cached Keychain material during the existing app
/// shutdown sequence.
pub(crate) fn clear() {
    let mut cache = session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.values.clear();
    cache.status = SessionStatus::Unverified;
    clear_backend_suppression(&mut cache);
}

#[cfg(not(test))]
fn backend_get_password(
    service: &str,
    account: &str,
) -> Result<Option<String>, BackendAccessError> {
    let entry = keyring::Entry::new(service, account).map_err(classify_keyring_error)?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(classify_keyring_error(error)),
    }
}

#[cfg(all(not(test), target_os = "macos"))]
fn backend_password_exists(service: &str, account: &str) -> Result<bool, BackendAccessError> {
    use security_framework::item::{ItemClass, ItemSearchOptions};
    use security_framework::os::macos::keychain::SecKeychain;

    // `kSecUseAuthenticationUISkip` is not sufficient for every legacy
    // login-keychain item on macOS. Hold the process-wide interaction guard
    // around this attribute-only lookup so readiness can never open a password
    // sheet. `password_exists` serializes this guard with real secret reads.
    let _interaction_guard =
        SecKeychain::disable_user_interaction().map_err(|error| BackendAccessError {
            suppress_session: macos_status_suppresses_session(error.code()),
            message: error.to_string(),
        })?;

    let mut search = ItemSearchOptions::new();
    search
        .class(ItemClass::generic_password())
        .service(service)
        .account(account)
        .load_attributes(true)
        .limit(1_i64)
        .skip_authenticated_items(true);
    match search.search() {
        Ok(matches) => Ok(!matches.is_empty()),
        Err(error) if error.code() == -25300 => Ok(false), // errSecItemNotFound
        Err(error) => Err(BackendAccessError {
            suppress_session: macos_status_suppresses_session(error.code()),
            message: error.to_string(),
        }),
    }
}

#[cfg(all(not(test), not(target_os = "macos")))]
fn backend_password_exists(service: &str, account: &str) -> Result<bool, BackendAccessError> {
    backend_get_password(service, account).map(|secret| secret.is_some())
}

#[cfg(not(test))]
fn backend_set_password(
    service: &str,
    account: &str,
    secret: &str,
) -> Result<(), BackendAccessError> {
    keyring::Entry::new(service, account)
        .map_err(classify_keyring_error)?
        .set_password(secret)
        .map_err(classify_keyring_error)
}

#[cfg(not(test))]
fn backend_delete_password(service: &str, account: &str) -> Result<(), BackendAccessError> {
    let entry = keyring::Entry::new(service, account).map_err(classify_keyring_error)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(classify_keyring_error(error)),
    }
}

#[cfg(not(test))]
fn classify_keyring_error(error: keyring::Error) -> BackendAccessError {
    let suppress_session = match &error {
        keyring::Error::NoStorageAccess(_) => true,
        #[cfg(target_os = "macos")]
        keyring::Error::PlatformFailure(source) => source
            .downcast_ref::<security_framework::base::Error>()
            .map(|error| macos_status_suppresses_session(error.code()))
            .unwrap_or(false),
        _ => false,
    };
    BackendAccessError {
        message: error.to_string(),
        suppress_session,
    }
}

#[cfg(target_os = "macos")]
fn macos_status_suppresses_session(code: i32) -> bool {
    matches!(
        code,
        -128     // errSecUserCanceled
            | -25243 // errSecNoAccessForItem
            | -25291 // errSecNotAvailable
            | -25293 // errSecAuthFailed
            | -25308 // errSecInteractionNotAllowed
            | -25315 // errSecInteractionRequired
    )
}

#[cfg(test)]
#[derive(Default, Clone, Copy)]
struct BackendCounts {
    reads: usize,
    writes: usize,
    deletes: usize,
}

#[cfg(test)]
#[derive(Default)]
struct TestBackend {
    values: HashMap<CacheKey, String>,
    counts: HashMap<CacheKey, BackendCounts>,
    read_failures: std::collections::HashSet<CacheKey>,
}

#[cfg(test)]
fn test_backend() -> &'static Mutex<TestBackend> {
    static BACKEND: OnceLock<Mutex<TestBackend>> = OnceLock::new();
    BACKEND.get_or_init(|| Mutex::new(TestBackend::default()))
}

#[cfg(test)]
fn backend_get_password(
    service: &str,
    account: &str,
) -> Result<Option<String>, BackendAccessError> {
    let key = cache_key(service, account);
    let mut backend = test_backend()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    backend.counts.entry(key.clone()).or_default().reads += 1;
    if backend.read_failures.contains(&key) {
        return Err(BackendAccessError {
            message: "simulated_keychain_access_denied".to_string(),
            suppress_session: true,
        });
    }
    Ok(backend.values.get(&key).cloned())
}

#[cfg(test)]
fn backend_password_exists(service: &str, account: &str) -> Result<bool, BackendAccessError> {
    Ok(test_backend()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values
        .contains_key(&cache_key(service, account)))
}

#[cfg(test)]
fn backend_set_password(
    service: &str,
    account: &str,
    secret: &str,
) -> Result<(), BackendAccessError> {
    let key = cache_key(service, account);
    let mut backend = test_backend()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    backend.counts.entry(key.clone()).or_default().writes += 1;
    backend.values.insert(key, secret.to_string());
    Ok(())
}

#[cfg(test)]
fn backend_delete_password(service: &str, account: &str) -> Result<(), BackendAccessError> {
    let key = cache_key(service, account);
    let mut backend = test_backend()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    backend.counts.entry(key.clone()).or_default().deletes += 1;
    backend.values.remove(&key);
    Ok(())
}

#[cfg(test)]
fn backend_counts(service: &str, account: &str) -> BackendCounts {
    test_backend()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .counts
        .get(&cache_key(service, account))
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn backend_read_count_for_test(service: &str, account: &str) -> usize {
    backend_counts(service, account).reads
}

#[cfg(test)]
pub(crate) fn evict_for_test(service: &str, account: &str) {
    session_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values
        .remove(&cache_key(service, account));
}

#[cfg(test)]
pub(crate) fn remove_backend_value_for_test(service: &str, account: &str) {
    test_backend()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values
        .remove(&cache_key(service, account));
}

#[cfg(test)]
fn fail_backend_reads_for_test(service: &str, account: &str) {
    test_backend()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .read_failures
        .insert(cache_key(service, account));
    evict_for_test(service, account);
}

#[cfg(test)]
fn allow_backend_reads_for_test(service: &str, account: &str) {
    test_backend()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .read_failures
        .remove(&cache_key(service, account));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keychain_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn all_noninteractive_and_denied_macos_statuses_suppress_the_session() {
        for code in [-128, -25243, -25291, -25293, -25308, -25315] {
            assert!(macos_status_suppresses_session(code), "status {code}");
        }
        assert!(!macos_status_suppresses_session(-25300));
    }

    fn begin_keychain_test() -> std::sync::MutexGuard<'static, ()> {
        let guard = keychain_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear();
        guard
    }

    #[test]
    fn repeated_reads_touch_the_backend_once() {
        let _guard = begin_keychain_test();
        let service = "cache-read-service";
        let account = "cache-read-account";
        set_password(service, account, "secret").unwrap();
        evict_for_test(service, account);

        assert_eq!(
            get_password(service, account).unwrap().as_deref(),
            Some("secret")
        );
        assert_eq!(
            get_password(service, account).unwrap().as_deref(),
            Some("secret")
        );
        assert_eq!(backend_counts(service, account).reads, 1);
    }

    #[test]
    fn successful_write_refreshes_the_cache_without_a_backend_read() {
        let _guard = begin_keychain_test();
        let service = "cache-write-service";
        let account = "cache-write-account";
        set_password(service, account, "first").unwrap();
        set_password(service, account, "second").unwrap();

        assert_eq!(
            get_password(service, account).unwrap().as_deref(),
            Some("second")
        );
        let counts = backend_counts(service, account);
        assert_eq!(counts.writes, 2);
        assert_eq!(counts.reads, 0);
    }

    #[test]
    fn successful_delete_caches_absence_without_a_backend_read() {
        let _guard = begin_keychain_test();
        let service = "cache-delete-service";
        let account = "cache-delete-account";
        set_password(service, account, "secret").unwrap();
        delete_password(service, account).unwrap();

        assert_eq!(get_password(service, account).unwrap(), None);
        let counts = backend_counts(service, account);
        assert_eq!(counts.deletes, 1);
        assert_eq!(counts.reads, 0);
    }

    #[test]
    fn denied_read_is_tombstoned_instead_of_prompting_again() {
        let _guard = begin_keychain_test();
        let service = "cache-denied-service";
        let account = "cache-denied-account";
        fail_backend_reads_for_test(service, account);

        assert!(get_password(service, account).is_err());
        assert!(get_password(service, account).is_err());
        assert_eq!(backend_counts(service, account).reads, 1);
    }

    #[test]
    fn denied_read_latches_the_backend_for_every_uncached_key() {
        let _guard = begin_keychain_test();
        let denied_service = "cache-global-denied-service";
        let denied_account = "cache-global-denied-account";
        let other_service = "cache-global-other-service";
        let other_account = "cache-global-other-account";
        fail_backend_reads_for_test(denied_service, denied_account);

        assert!(get_password(denied_service, denied_account).is_err());
        assert!(get_password(other_service, other_account).is_err());
        assert!(set_password(other_service, other_account, "secret").is_err());
        assert!(delete_password(other_service, other_account).is_err());
        assert_eq!(backend_counts(denied_service, denied_account).reads, 1);
        assert_eq!(backend_counts(other_service, other_account).reads, 0);
        assert_eq!(backend_counts(other_service, other_account).writes, 0);
        assert_eq!(backend_counts(other_service, other_account).deletes, 0);
    }

    #[test]
    fn explicit_retry_recovers_once_without_enabling_prompt_loops() {
        let _guard = begin_keychain_test();
        let service = "cache-retry-service";
        let account = "cache-retry-account";
        set_password(service, account, "secret").unwrap();
        fail_backend_reads_for_test(service, account);

        assert!(get_password(service, account).is_err());
        allow_backend_reads_for_test(service, account);
        assert_eq!(
            retry_password(service, account).unwrap().as_deref(),
            Some("secret")
        );
        assert_eq!(
            get_password(service, account).unwrap().as_deref(),
            Some("secret")
        );
        assert_eq!(backend_counts(service, account).reads, 2);
    }
}
