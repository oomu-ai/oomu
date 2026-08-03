use crate::{
    agentic_loop,
    db::PersistenceEngine,
    foundation::clock::unix_time_ms_i64 as unix_time_ms,
    gemma::GemmaService,
    security::mods::{self, ModPermissions},
    sovereign_identity::SovereignIdentity,
};
use notify::{Event as NotifyEvent, EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::Manager;

const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);
const MAX_HOOK_PAYLOAD_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModHookEventPayload {
    pub mod_id: String,
    pub watch_path: String,
    pub source_path: String,
    pub raw_content: String,
    pub detected_at_ms: i64,
}

pub trait HookEventDispatcher: Send + Sync + 'static {
    fn dispatch(&self, payload: ModHookEventPayload) -> Result<(), HookRegistryError>;
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum HookRegistryError {
    #[error("Unable to read active mod hook manifests: {0}")]
    Database(String),
    #[error("Invalid event hook manifest for mod {mod_id}: {reason}")]
    Manifest { mod_id: String, reason: String },
    #[error("Mod {mod_id} event hook path {path} is outside its approved sandbox: {reason}")]
    Permission {
        mod_id: String,
        path: String,
        reason: String,
    },
    #[error("Invalid watch path for mod {mod_id}: {path}: {reason}")]
    WatchPath {
        mod_id: String,
        path: String,
        reason: String,
    },
    #[error("Native watcher failed for mod {mod_id} at {path}: {reason}")]
    Watcher {
        mod_id: String,
        path: String,
        reason: String,
    },
    #[error("Unable to read event file {path}: {reason}")]
    Read { path: String, reason: String },
    #[error("Event payload at {path} exceeded {max_bytes} bytes")]
    PayloadTooLarge { path: String, max_bytes: u64 },
    #[error("Background hook dispatcher failed: {0}")]
    Dispatch(String),
}

impl HookRegistryError {
    fn code(&self) -> &'static str {
        match self {
            Self::Database(_) => "database_unavailable",
            Self::Manifest { .. } => "manifest_invalid",
            Self::Permission { .. } => "permission_denied",
            Self::WatchPath { .. } => "watch_path_invalid",
            Self::Watcher { .. } => "watcher_unavailable",
            Self::Read { .. } => "event_read_failed",
            Self::PayloadTooLarge { .. } => "payload_too_large",
            Self::Dispatch(_) => "dispatch_failed",
        }
    }
}

#[derive(Clone, Default)]
pub struct BackgroundHookRegistry {
    inner: Arc<Mutex<RegistryState>>,
}

#[derive(Default)]
struct RegistryState {
    watchers: HashMap<HookWatchKey, ModDirectoryWatcherHandle>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct HookWatchKey {
    mod_id: String,
    watch_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModEventHookSpec {
    mod_id: String,
    watch_path: PathBuf,
}

pub struct ModDirectoryWatcherHandle {
    stop_tx: Sender<WatcherMessage>,
    join_handle: Option<thread::JoinHandle<()>>,
}

enum WatcherMessage {
    Notify(notify::Result<NotifyEvent>),
    Stop,
}

impl ModDirectoryWatcherHandle {
    pub fn stop(mut self) {
        let _ = self.stop_tx.send(WatcherMessage::Stop);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl BackgroundHookRegistry {
    pub fn refresh_active_mod_hooks(
        &self,
        persistence: &PersistenceEngine,
        dispatcher: Arc<dyn HookEventDispatcher>,
    ) -> Result<usize, HookRegistryError> {
        let specs = active_mod_event_hook_specs(persistence)?;
        let desired_keys = specs
            .iter()
            .map(|spec| HookWatchKey {
                mod_id: spec.mod_id.clone(),
                watch_path: spec.watch_path.clone(),
            })
            .collect::<HashSet<_>>();

        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let stale = state
            .watchers
            .keys()
            .filter(|key| !desired_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(handle) = state.watchers.remove(&key) {
                handle.stop();
            }
        }

        for spec in specs {
            let key = HookWatchKey {
                mod_id: spec.mod_id.clone(),
                watch_path: spec.watch_path.clone(),
            };
            if state.watchers.contains_key(&key) {
                continue;
            }
            let handle = spawn_mod_directory_watcher_with_dispatcher(
                spec.mod_id.clone(),
                spec.watch_path.clone(),
                Arc::clone(&dispatcher),
                DEFAULT_DEBOUNCE,
            )
            .map_err(|error| HookRegistryError::Watcher {
                mod_id: spec.mod_id,
                path: spec.watch_path.display().to_string(),
                reason: error.to_string(),
            })?;
            state.watchers.insert(key, handle);
        }

        Ok(state.watchers.len())
    }

    pub fn clear_active_mod_hooks(&self) {
        let handles = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .watchers
                .drain()
                .map(|(_, handle)| handle)
                .collect::<Vec<_>>()
        };
        for handle in handles {
            handle.stop();
        }
    }

    #[cfg(test)]
    fn active_watcher_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .watchers
            .len()
    }

    #[cfg(test)]
    fn shutdown_all(&self) {
        self.clear_active_mod_hooks();
    }
}

pub(crate) fn refresh_active_mod_hook_registry_async(
    app: tauri::AppHandle,
    registry: BackgroundHookRegistry,
    persistence: PersistenceEngine,
    gemma: GemmaService,
    identity: SovereignIdentity,
    safe_mode: bool,
) {
    if safe_mode {
        tauri::async_runtime::spawn_blocking(move || {
            registry.clear_active_mod_hooks();
            eprintln!("OOMU_BACKGROUND_HOOK_REGISTRY_SAFE_MODE third_party_hooks=blocked");
            app.state::<crate::DegradedModeState>()
                .clear_after_verified_recovery(
                    "backgroundHooks",
                    crate::persistence_health::BackingStoreClass::NotApplicable,
                    "Safe mode verified the background hook registry is empty.",
                );
        });
        return;
    }

    let dispatcher = Arc::new(AgenticLoopHookDispatcher {
        persistence: persistence.clone(),
        gemma,
        identity,
    });
    tauri::async_runtime::spawn_blocking(move || {
        match registry.refresh_active_mod_hooks(&persistence, dispatcher) {
            Ok(count) => {
                eprintln!("OOMU_BACKGROUND_HOOK_REGISTRY_READY active_watchers={count}");
                app.state::<crate::DegradedModeState>()
                    .clear_after_verified_recovery(
                        "backgroundHooks",
                        crate::persistence_health::BackingStoreClass::NotApplicable,
                        format!(
                            "Background hook refresh succeeded with {count} active watcher(s)."
                        ),
                    );
            }
            Err(error) => {
                eprintln!("OOMU_BACKGROUND_HOOK_REGISTRY_FAILED code={}", error.code());
                app.state::<crate::DegradedModeState>().activate(
                    "backgroundHooks",
                    format!(
                        "Background hook registry refresh failed ({}).",
                        error.code()
                    ),
                    crate::persistence_health::BackingStoreClass::NotApplicable,
                    true,
                    "Background mod hooks are unavailable until their registry refresh succeeds.",
                );
            }
        }
    });
}

fn spawn_mod_directory_watcher_with_dispatcher(
    mod_id: String,
    watch_path: PathBuf,
    dispatcher: Arc<dyn HookEventDispatcher>,
    debounce_window: Duration,
) -> Result<ModDirectoryWatcherHandle, notify::Error> {
    let canonical_watch_path = fs::canonicalize(&watch_path).unwrap_or(watch_path.clone());
    let (tx, rx) = mpsc::channel::<WatcherMessage>();
    let notify_tx = tx.clone();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = notify_tx.send(WatcherMessage::Notify(event));
    })?;
    watcher.watch(&watch_path, RecursiveMode::NonRecursive)?;

    let thread_mod_id = mod_id.clone();
    let join_handle = thread::spawn(move || {
        let _watcher = watcher;
        let mut pending = HashMap::<PathBuf, Instant>::new();
        loop {
            let message = next_watcher_message(&rx, &pending, debounce_window);
            match message {
                Ok(Some(WatcherMessage::Notify(Ok(event)))) => {
                    if is_write_like_event(&event.kind) {
                        let now = Instant::now();
                        for path in event.paths {
                            pending.insert(path, now);
                        }
                    }
                }
                Ok(Some(WatcherMessage::Notify(Err(error)))) => {
                    let _ = error;
                    eprintln!("OOMU_BACKGROUND_HOOK_WATCHER_ERROR code=notify_failed");
                }
                Ok(Some(WatcherMessage::Stop)) => break,
                Ok(None) => dispatch_due_events(
                    &thread_mod_id,
                    &canonical_watch_path,
                    &dispatcher,
                    &mut pending,
                    debounce_window,
                ),
                Err(_) => break,
            }
        }
        dispatch_all_pending_events(
            &thread_mod_id,
            &canonical_watch_path,
            &dispatcher,
            &mut pending,
        );
    });

    Ok(ModDirectoryWatcherHandle {
        stop_tx: tx,
        join_handle: Some(join_handle),
    })
}

fn next_watcher_message(
    rx: &mpsc::Receiver<WatcherMessage>,
    pending: &HashMap<PathBuf, Instant>,
    debounce_window: Duration,
) -> Result<Option<WatcherMessage>, RecvTimeoutError> {
    if pending.is_empty() {
        return rx
            .recv()
            .map(Some)
            .map_err(|_| RecvTimeoutError::Disconnected);
    }

    let now = Instant::now();
    let timeout = pending
        .values()
        .map(|last_seen| {
            debounce_window
                .checked_sub(now.saturating_duration_since(*last_seen))
                .unwrap_or(Duration::ZERO)
        })
        .min()
        .unwrap_or(debounce_window);

    match rx.recv_timeout(timeout) {
        Ok(message) => Ok(Some(message)),
        Err(RecvTimeoutError::Timeout) => Ok(None),
        Err(error) => Err(error),
    }
}

fn dispatch_due_events(
    mod_id: &str,
    watch_path: &Path,
    dispatcher: &Arc<dyn HookEventDispatcher>,
    pending: &mut HashMap<PathBuf, Instant>,
    debounce_window: Duration,
) {
    let now = Instant::now();
    let due = pending
        .iter()
        .filter_map(|(path, last_seen)| {
            (now.saturating_duration_since(*last_seen) >= debounce_window).then(|| path.clone())
        })
        .collect::<Vec<_>>();
    for path in due {
        pending.remove(&path);
        if let Err(error) = handle_mod_write_event(mod_id, watch_path, &path, dispatcher) {
            eprintln!(
                "OOMU_BACKGROUND_HOOK_DISPATCH_FAILED code={} source=opaque_file",
                error.code()
            );
        }
    }
}

fn dispatch_all_pending_events(
    mod_id: &str,
    watch_path: &Path,
    dispatcher: &Arc<dyn HookEventDispatcher>,
    pending: &mut HashMap<PathBuf, Instant>,
) {
    let paths = pending.keys().cloned().collect::<Vec<_>>();
    pending.clear();
    for path in paths {
        if let Err(error) = handle_mod_write_event(mod_id, watch_path, &path, dispatcher) {
            eprintln!(
                "OOMU_BACKGROUND_HOOK_DISPATCH_FAILED code={} source=opaque_file",
                error.code()
            );
        }
    }
}

fn handle_mod_write_event(
    mod_id: &str,
    watch_path: &Path,
    event_path: &Path,
    dispatcher: &Arc<dyn HookEventDispatcher>,
) -> Result<(), HookRegistryError> {
    let canonical_file = fs::canonicalize(event_path).map_err(|error| HookRegistryError::Read {
        path: event_path.display().to_string(),
        reason: error.to_string(),
    })?;
    if !canonical_file.starts_with(watch_path) {
        return Err(HookRegistryError::Permission {
            mod_id: mod_id.to_string(),
            path: canonical_file.display().to_string(),
            reason: format!(
                "resolved path is outside watched directory {}",
                watch_path.display()
            ),
        });
    }
    let metadata = fs::metadata(&canonical_file).map_err(|error| HookRegistryError::Read {
        path: canonical_file.display().to_string(),
        reason: error.to_string(),
    })?;
    if !metadata.is_file() {
        return Ok(());
    }

    let raw_content = read_stable_file_payload(&canonical_file)?;
    dispatcher.dispatch(ModHookEventPayload {
        mod_id: mod_id.to_string(),
        watch_path: watch_path.display().to_string(),
        source_path: canonical_file.display().to_string(),
        raw_content,
        detected_at_ms: unix_time_ms(),
    })
}

fn read_stable_file_payload(path: &Path) -> Result<String, HookRegistryError> {
    wait_for_file_settle(path)?;
    let file =
        OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|error| HookRegistryError::Read {
                path: path.display().to_string(),
                reason: error.to_string(),
            })?;
    let _lock = SharedFileLock::acquire(&file).map_err(|error| HookRegistryError::Read {
        path: path.display().to_string(),
        reason: format!("shared file lock failed: {error}"),
    })?;
    let metadata = file.metadata().map_err(|error| HookRegistryError::Read {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    if metadata.len() > MAX_HOOK_PAYLOAD_BYTES {
        return Err(HookRegistryError::PayloadTooLarge {
            path: path.display().to_string(),
            max_bytes: MAX_HOOK_PAYLOAD_BYTES,
        });
    }

    let mut bytes = Vec::new();
    let file_ref = &file;
    let mut limited = file_ref.take(MAX_HOOK_PAYLOAD_BYTES + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|error| HookRegistryError::Read {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    if bytes.len() as u64 > MAX_HOOK_PAYLOAD_BYTES {
        return Err(HookRegistryError::PayloadTooLarge {
            path: path.display().to_string(),
            max_bytes: MAX_HOOK_PAYLOAD_BYTES,
        });
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn wait_for_file_settle(path: &Path) -> Result<(), HookRegistryError> {
    let mut last = None;
    for _ in 0..5 {
        let metadata = fs::metadata(path).map_err(|error| HookRegistryError::Read {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        let snapshot = (
            metadata.len(),
            metadata.modified().ok(),
            metadata.created().ok(),
        );
        if last.as_ref().is_some_and(|prior| prior == &snapshot) {
            return Ok(());
        }
        last = Some(snapshot);
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

struct SharedFileLock<'a> {
    file: &'a File,
}

impl<'a> SharedFileLock<'a> {
    #[cfg(target_os = "macos")]
    fn acquire(file: &'a File) -> io::Result<Self> {
        use std::os::fd::AsRawFd;

        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH) };
        if result == 0 {
            Ok(Self { file })
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn acquire(file: &'a File) -> io::Result<Self> {
        Ok(Self { file })
    }
}

impl Drop for SharedFileLock<'_> {
    #[cfg(target_os = "macos")]
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }

    #[cfg(not(target_os = "macos"))]
    fn drop(&mut self) {
        let _ = self.file;
    }
}

fn is_write_like_event(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Any) || kind.is_create() || kind.is_modify()
}

fn active_mod_event_hook_specs(
    persistence: &PersistenceEngine,
) -> Result<Vec<ModEventHookSpec>, HookRegistryError> {
    let records =
        mods::active_mod_manifest_records(persistence).map_err(HookRegistryError::Database)?;
    let mut specs = Vec::new();
    for record in records {
        match event_hook_specs_from_manifest(&record.id, &record.manifest_json) {
            Ok(mut record_specs) => specs.append(&mut record_specs),
            Err(error) => {
                eprintln!(
                    "OOMU_BACKGROUND_HOOK_MANIFEST_SKIPPED code={}",
                    error.code()
                )
            }
        }
    }
    Ok(specs)
}

fn event_hook_specs_from_manifest(
    mod_id: &str,
    manifest_json: &Value,
) -> Result<Vec<ModEventHookSpec>, HookRegistryError> {
    let hooks = manifest_json.get("hooks").unwrap_or(&Value::Null);
    let raw_paths = event_hook_paths(hooks);
    if raw_paths.is_empty() {
        return Ok(Vec::new());
    }

    let permissions = manifest_json
        .get("permissions")
        .filter(|value| value.is_object())
        .cloned()
        .map(serde_json::from_value::<ModPermissions>)
        .transpose()
        .map_err(|error| HookRegistryError::Manifest {
            mod_id: mod_id.to_string(),
            reason: format!("permissions must use structured snake_case fields: {error}"),
        })?
        .ok_or_else(|| HookRegistryError::Permission {
            mod_id: mod_id.to_string(),
            path: raw_paths.join(", "),
            reason: "event hooks require structured permissions.allowed_paths".to_string(),
        })?;

    let mut specs = Vec::new();
    let mut seen = HashSet::new();
    for raw_path in raw_paths {
        let raw_path = raw_path.trim();
        if raw_path.is_empty() {
            continue;
        }
        let path = PathBuf::from(raw_path);
        if !path.is_absolute() {
            return Err(HookRegistryError::WatchPath {
                mod_id: mod_id.to_string(),
                path: raw_path.to_string(),
                reason: "event hook watch paths must be absolute".to_string(),
            });
        }
        mods::validate_mod_filesystem_access(mod_id, &path, &permissions).map_err(|error| {
            HookRegistryError::Permission {
                mod_id: mod_id.to_string(),
                path: raw_path.to_string(),
                reason: error.to_string(),
            }
        })?;
        let canonical_path =
            fs::canonicalize(&path).map_err(|error| HookRegistryError::WatchPath {
                mod_id: mod_id.to_string(),
                path: raw_path.to_string(),
                reason: error.to_string(),
            })?;
        let metadata =
            fs::metadata(&canonical_path).map_err(|error| HookRegistryError::WatchPath {
                mod_id: mod_id.to_string(),
                path: canonical_path.display().to_string(),
                reason: error.to_string(),
            })?;
        if !metadata.is_dir() {
            return Err(HookRegistryError::WatchPath {
                mod_id: mod_id.to_string(),
                path: canonical_path.display().to_string(),
                reason: "event hooks may watch directories only".to_string(),
            });
        }
        if seen.insert(canonical_path.clone()) {
            specs.push(ModEventHookSpec {
                mod_id: mod_id.to_string(),
                watch_path: canonical_path,
            });
        }
    }
    Ok(specs)
}

fn event_hook_paths(hooks: &Value) -> Vec<String> {
    ["event_hook", "event_hooks", "events"]
        .into_iter()
        .filter_map(|key| hooks.get(key))
        .flat_map(paths_from_hook_value)
        .collect()
}

fn paths_from_hook_value(value: &Value) -> Vec<String> {
    match value {
        Value::String(path) => vec![path.to_string()],
        Value::Array(items) => items.iter().flat_map(paths_from_hook_value).collect(),
        Value::Object(map) => {
            if map.get("enabled").and_then(Value::as_bool) == Some(false) {
                return Vec::new();
            }
            let mut paths = Vec::new();
            for key in [
                "watch_path",
                "watchPath",
                "path",
                "directory",
                "dir",
                "watch_paths",
                "watchPaths",
                "paths",
            ] {
                if let Some(value) = map.get(key) {
                    paths.extend(paths_from_hook_value(value));
                }
            }
            paths
        }
        _ => Vec::new(),
    }
}

#[derive(Clone)]
struct AgenticLoopHookDispatcher {
    persistence: PersistenceEngine,
    gemma: GemmaService,
    identity: SovereignIdentity,
}

impl HookEventDispatcher for AgenticLoopHookDispatcher {
    fn dispatch(&self, payload: ModHookEventPayload) -> Result<(), HookRegistryError> {
        let persistence = self.persistence.clone();
        let gemma = self.gemma.clone();
        let identity = self.identity.clone();
        tauri::async_runtime::spawn(async move {
            let event = agentic_loop::BackgroundHookObjective {
                mod_id: payload.mod_id,
                source_path: payload.source_path,
                raw_content: payload.raw_content,
                detected_at_ms: payload.detected_at_ms,
            };
            match agentic_loop::process_background_hook_objective(
                event,
                gemma,
                persistence,
                identity,
            )
            .await
            {
                Ok(plan) => eprintln!(
                    "OOMU_BACKGROUND_HOOK_AGENT_PLAN_READY plan_id={} objective_chars={}",
                    plan.id,
                    plan.objective.chars().count()
                ),
                Err(error) => eprintln!(
                    "OOMU_BACKGROUND_HOOK_AGENT_PLAN_FAILED code={} boundary={}",
                    crate::redaction::redacted_log_text(error.code),
                    crate::redaction::redacted_log_text(error.boundary)
                ),
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::PersistenceEngine;
    use rusqlite::params;
    use serde_json::json;
    use std::{
        fs::File,
        io::Write,
        sync::mpsc::Receiver,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    #[derive(Clone)]
    struct ChannelDispatcher {
        tx: Sender<ModHookEventPayload>,
    }

    impl HookEventDispatcher for ChannelDispatcher {
        fn dispatch(&self, payload: ModHookEventPayload) -> Result<(), HookRegistryError> {
            self.tx
                .send(payload)
                .map_err(|error| HookRegistryError::Dispatch(error.to_string()))
        }
    }

    fn test_temp_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "oomu_background_hooks_{name}_{}_{}",
            std::process::id(),
            suffix
        ));
        fs::create_dir_all(&path).expect("test directory created");
        path
    }

    fn payload_receiver() -> (Arc<dyn HookEventDispatcher>, Receiver<ModHookEventPayload>) {
        let (tx, rx) = mpsc::channel();
        (Arc::new(ChannelDispatcher { tx }), rx)
    }

    #[test]
    fn verify_file_watcher_trigger() {
        let temp_dir = test_temp_dir("watcher_trigger");
        let watch_path = temp_dir.join("inbox");
        fs::create_dir_all(&watch_path).expect("watch directory created");
        let mod_id = "ai.eldris.mods.test_watcher".to_string();
        let (dispatcher, rx) = payload_receiver();

        let handle = spawn_mod_directory_watcher_with_dispatcher(
            mod_id.clone(),
            watch_path.clone(),
            dispatcher,
            Duration::from_millis(100),
        )
        .expect("watcher spawns");

        let file_path = watch_path.join("customer_transcript.json");
        let mut file = File::create(&file_path).expect("mock transcript created");
        writeln!(
            file,
            r#"{{"customer_id": "9982", "message": "My machine will not start."}}"#
        )
        .expect("mock transcript written");
        file.sync_all().expect("mock transcript synced");
        drop(file);

        let payload = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("background event dispatches within threshold");
        handle.stop();

        assert_eq!(payload.mod_id, mod_id);
        assert_eq!(
            PathBuf::from(payload.watch_path),
            fs::canonicalize(&watch_path).unwrap()
        );
        assert_eq!(
            PathBuf::from(payload.source_path),
            fs::canonicalize(&file_path).unwrap()
        );
        assert!(payload.raw_content.contains("\"customer_id\": \"9982\""));
        assert!(payload.raw_content.contains("My machine will not start."));
    }

    #[test]
    fn event_hook_manifest_requires_allowed_path_sandbox() {
        let temp_dir = test_temp_dir("manifest_gate");
        let allowed_dir = temp_dir.join("allowed");
        let denied_dir = temp_dir.join("denied");
        fs::create_dir_all(&allowed_dir).expect("allowed dir created");
        fs::create_dir_all(&denied_dir).expect("denied dir created");
        let manifest = json!({
            "permissions": {
                "allowed_paths": [allowed_dir.display().to_string()]
            },
            "hooks": {
                "event_hook": {
                    "watch_path": denied_dir.display().to_string()
                }
            }
        });

        let denied = event_hook_specs_from_manifest("ai.eldris.mods.denied", &manifest)
            .expect_err("watch path outside allowed_paths is rejected");
        assert!(matches!(denied, HookRegistryError::Permission { .. }));
    }

    #[test]
    fn registry_starts_only_active_event_hook_mods() {
        let temp_dir = test_temp_dir("registry");
        let db_path = temp_dir.join("state.sqlite");
        let watch_path = temp_dir.join("active_watch");
        let installed_path = temp_dir.join("installed_mod");
        fs::create_dir_all(&watch_path).expect("watch dir created");
        fs::create_dir_all(&installed_path).expect("installed mod dir created");
        let engine = PersistenceEngine::initialize_at(db_path).expect("test db initializes");
        let connection = engine.open_connection().expect("connection opens");
        connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS installed_mods (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL,
                    is_active INTEGER NOT NULL DEFAULT 0,
                    version TEXT NOT NULL,
                    author TEXT NOT NULL,
                    category TEXT NOT NULL,
                    package_size TEXT NOT NULL,
                    last_updated TEXT NOT NULL,
                    permissions_json TEXT NOT NULL DEFAULT '[]',
                    endpoints_json TEXT NOT NULL DEFAULT '[]',
                    installed_path TEXT NOT NULL,
                    manifest_json TEXT NOT NULL,
                    default_system_prompt TEXT,
                    entrypoint TEXT NOT NULL,
                    installed_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                );
                ",
            )
            .expect("installed mods schema exists");
        let manifest = json!({
            "id": "ai.eldris.mods.registry",
            "name": "Watcher",
            "version": "1.0.0",
            "author": "OOMU",
            "description": "Event watcher",
            "entrypoint": "index.js",
            "permissions": {
                "allowed_paths": [watch_path.display().to_string()]
            },
            "hooks": {
                "event_hooks": [
                    {"watch_path": watch_path.display().to_string()}
                ]
            }
        });
        fs::write(
            installed_path.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
        )
        .expect("installed manifest written");
        fs::write(installed_path.join("index.js"), b"export default {};\n")
            .expect("installed entrypoint written");
        connection
            .execute(
                "
                INSERT INTO installed_mods (
                    id, name, description, is_active, version, author, category,
                    package_size, last_updated, permissions_json, endpoints_json,
                    installed_path, manifest_json, default_system_prompt, entrypoint,
                    installed_at_ms, updated_at_ms
                )
                VALUES (?1, 'Watcher', 'Event watcher', 1, '1.0.0', 'OOMU', 'Event Hook',
                        '1 KB', 'June 27, 2026', '[]', '[]', ?2, ?3, NULL, 'index.js', 1, 1)
                ",
                params![
                    "ai.eldris.mods.registry",
                    installed_path.display().to_string(),
                    manifest.to_string()
                ],
            )
            .expect("active mod inserted");

        let registry = BackgroundHookRegistry::default();
        let (dispatcher, _rx) = payload_receiver();
        let count = registry
            .refresh_active_mod_hooks(&engine, dispatcher)
            .expect("registry refreshes");
        assert_eq!(count, 1);
        assert_eq!(registry.active_watcher_count(), 1);
        registry.shutdown_all();
    }
}
