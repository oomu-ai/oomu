use crate::foundation::clock::unix_time_ms_i64 as unix_time_ms;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackingStoreClass {
    NotApplicable,
    Persistent,
    RecoveryPending,
    Volatile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryProbeResult {
    pub attempted_at_ms: i64,
    pub succeeded: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemHealthStatus {
    pub subsystem: String,
    pub active: bool,
    pub cause: Option<String>,
    pub first_occurred_at_ms: Option<i64>,
    pub backing_store_class: BackingStoreClass,
    pub recovery_eligible: bool,
    pub last_probe_result: Option<RecoveryProbeResult>,
    pub user_visible_impact: String,
}

impl SubsystemHealthStatus {
    fn healthy(
        subsystem: impl Into<String>,
        backing_store_class: BackingStoreClass,
        user_visible_impact: impl Into<String>,
    ) -> Self {
        Self {
            subsystem: subsystem.into(),
            active: false,
            cause: None,
            first_occurred_at_ms: None,
            backing_store_class,
            recovery_eligible: false,
            last_probe_result: None,
            user_visible_impact: sanitize_health_text(&user_visible_impact.into(), 512),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DegradedModeStatus {
    pub active: bool,
    pub reason: Option<String>,
    pub has_volatile_storage: bool,
    pub subsystems: Vec<SubsystemHealthStatus>,
}

#[derive(Default)]
pub struct DegradedModeState {
    inner: Mutex<BTreeMap<String, SubsystemHealthStatus>>,
}

impl DegradedModeState {
    pub(crate) fn register_healthy(
        &self,
        subsystem: &str,
        backing_store_class: BackingStoreClass,
        user_visible_impact: impl Into<String>,
    ) {
        let mut states = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        states.entry(subsystem.to_string()).or_insert_with(|| {
            SubsystemHealthStatus::healthy(subsystem, backing_store_class, user_visible_impact)
        });
    }

    pub(crate) fn activate(
        &self,
        subsystem: &str,
        cause: impl Into<String>,
        backing_store_class: BackingStoreClass,
        recovery_eligible: bool,
        user_visible_impact: impl Into<String>,
    ) {
        let cause = sanitize_health_text(&cause.into(), 768);
        let impact = sanitize_health_text(&user_visible_impact.into(), 512);
        let mut states = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = states.entry(subsystem.to_string()).or_insert_with(|| {
            SubsystemHealthStatus::healthy(subsystem, backing_store_class, impact.clone())
        });
        state.active = true;
        state.cause = Some(cause);
        state.first_occurred_at_ms.get_or_insert_with(unix_time_ms);
        state.backing_store_class = backing_store_class;
        state.recovery_eligible = recovery_eligible;
        state.user_visible_impact = impact;
    }

    pub(crate) fn mark_recovery_pending(&self, subsystem: &str, message: impl Into<String>) {
        let message = message.into();
        let mut states = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(state) = states.get_mut(subsystem) {
            state.active = true;
            state.backing_store_class = BackingStoreClass::RecoveryPending;
            state.recovery_eligible = true;
            state.last_probe_result = Some(RecoveryProbeResult {
                attempted_at_ms: unix_time_ms(),
                succeeded: false,
                message: sanitize_health_text(&message, 768),
            });
        }
    }

    pub(crate) fn mark_reconciled_cleanup_pending(
        &self,
        subsystem: &str,
        message: impl Into<String>,
    ) {
        let mut states = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(state) = states.get_mut(subsystem) {
            state.active = true;
            state.cause = Some(
                "Durable storage is verified; encrypted volatile recovery artifacts await explicit cleanup."
                    .to_string(),
            );
            state.backing_store_class = BackingStoreClass::Persistent;
            state.recovery_eligible = true;
            state.last_probe_result = Some(RecoveryProbeResult {
                attempted_at_ms: unix_time_ms(),
                succeeded: true,
                message: sanitize_health_text(&message.into(), 768),
            });
        }
    }

    /// A subsystem may recover only itself and never while its backing store is volatile or pending.
    pub(crate) fn clear_after_verified_recovery(
        &self,
        subsystem: &str,
        backing_store_class: BackingStoreClass,
        probe_message: impl Into<String>,
    ) -> bool {
        if matches!(
            backing_store_class,
            BackingStoreClass::Volatile | BackingStoreClass::RecoveryPending
        ) {
            return false;
        }
        let mut states = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = states.get_mut(subsystem) else {
            return false;
        };
        state.active = false;
        state.cause = None;
        state.backing_store_class = backing_store_class;
        state.recovery_eligible = false;
        state.last_probe_result = Some(RecoveryProbeResult {
            attempted_at_ms: unix_time_ms(),
            succeeded: true,
            message: sanitize_health_text(&probe_message.into(), 768),
        });
        true
    }

    pub(crate) fn snapshot(&self) -> DegradedModeStatus {
        let states = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let subsystems = states.values().cloned().collect::<Vec<_>>();
        let active_states = subsystems
            .iter()
            .filter(|state| state.active)
            .collect::<Vec<_>>();
        DegradedModeStatus {
            active: !active_states.is_empty(),
            reason: (!active_states.is_empty()).then(|| {
                active_states
                    .iter()
                    .map(|state| {
                        format!(
                            "{}: {}",
                            state.subsystem,
                            state.cause.as_deref().unwrap_or("recovery required")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
            // RecoveryPending describes a capability that still needs a
            // successful probe. It does not mean writes left durable storage.
            // A retained volatile recovery session is added by
            // get_degraded_mode_status, which has direct session evidence.
            has_volatile_storage: subsystems.iter().any(|state| {
                state.active && state.backing_store_class == BackingStoreClass::Volatile
            }),
            subsystems,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolatileRecoveryStatus {
    pub session_id: String,
    pub created_at_ms: i64,
    pub reconciliation_verified: bool,
    pub cleanup_eligible: bool,
    #[serde(default)]
    pub requires_confirmation: bool,
    pub last_result: Option<String>,
}

#[derive(Debug)]
struct VolatileRecoveryProgress {
    reconciliation_verified: bool,
    cleanup_eligible: bool,
    requires_confirmation: bool,
    last_result: Option<String>,
}

#[derive(Debug)]
pub struct VolatileStoreSession {
    session_id: String,
    root: PathBuf,
    created_at_ms: i64,
    progress: Mutex<VolatileRecoveryProgress>,
}

pub struct VolatileStoreSessionManager {
    session: Mutex<Option<Arc<VolatileStoreSession>>>,
    base: PathBuf,
}

impl Default for VolatileStoreSessionManager {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            base: recovery_base_directory(),
        }
    }
}

impl VolatileStoreSessionManager {
    pub(crate) fn initialize() -> Result<Self, String> {
        Self::initialize_in(recovery_base_directory())
    }

    pub(crate) fn initialize_in(base: PathBuf) -> Result<Self, String> {
        Ok(Self {
            session: Mutex::new(discover_pending_session_in(&base)?.map(Arc::new)),
            base,
        })
    }

    pub(crate) fn get_or_create(&self) -> Result<Arc<VolatileStoreSession>, String> {
        let mut session = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = session.as_ref() {
            return Ok(Arc::clone(existing));
        }
        let created = Arc::new(VolatileStoreSession::create_in(&self.base)?);
        *session = Some(Arc::clone(&created));
        Ok(created)
    }

    pub(crate) fn current(&self) -> Option<Arc<VolatileStoreSession>> {
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(Arc::clone)
    }

    pub(crate) fn forget_cleaned(&self) -> Result<(), String> {
        let mut session = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if session
            .as_ref()
            .is_some_and(|current| !current.root().exists())
        {
            *session = discover_pending_session_in(&self.base)?.map(Arc::new);
        }
        Ok(())
    }

    /// Removes a freshly verified current session only after every other
    /// retained session has passed discovery validation. This keeps the
    /// verified session available for explicit cleanup if discovery encounters
    /// an incomplete or invalid successor.
    pub(crate) fn cleanup_current_and_advance(
        &self,
        expected: &Arc<VolatileStoreSession>,
    ) -> Result<(), String> {
        let mut current = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(active) = current.as_ref() else {
            return Err("No volatile persistence session is active.".to_string());
        };
        if !Arc::ptr_eq(active, expected) {
            return Err(
                "Volatile persistence session changed before verified cleanup.".to_string(),
            );
        }

        let next =
            discover_pending_session_in_excluding(&self.base, Some(expected.root()))?.map(Arc::new);
        expected.cleanup_after_reconciliation()?;
        *current = next;
        Ok(())
    }
}

impl VolatileStoreSession {
    pub(crate) fn create_in(base: &Path) -> Result<Self, String> {
        fs::create_dir_all(&base).map_err(|error| error.to_string())?;
        set_private_directory_permissions(&base)?;

        for _ in 0..8 {
            let session_id = random_hex(32);
            let root = base.join(&session_id);
            match fs::create_dir(&root) {
                Ok(()) => {
                    set_private_directory_permissions(&root)?;
                    let session = Self {
                        session_id,
                        root,
                        created_at_ms: unix_time_ms(),
                        progress: Mutex::new(VolatileRecoveryProgress {
                            reconciliation_verified: false,
                            cleanup_eligible: false,
                            requires_confirmation: false,
                            last_result: None,
                        }),
                    };
                    session.write_manifest()?;
                    return Ok(session);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Unable to allocate a unique private volatile-store session directory.".to_string())
    }

    pub(crate) fn path_for_file(&self, name: &str) -> Result<PathBuf, String> {
        validate_store_name(name)?;
        Ok(self.root.join(format!("{name}.sqlite")))
    }

    pub(crate) fn enforce_private_tree(&self) -> Result<(), String> {
        enforce_private_tree(&self.root)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn snapshot(&self) -> VolatileRecoveryStatus {
        let progress = self
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        VolatileRecoveryStatus {
            session_id: self.session_id.clone(),
            created_at_ms: self.created_at_ms,
            reconciliation_verified: progress.reconciliation_verified,
            cleanup_eligible: progress.cleanup_eligible,
            requires_confirmation: progress.requires_confirmation,
            last_result: progress.last_result.clone(),
        }
    }

    pub(crate) fn record_reconciliation(
        &self,
        succeeded: bool,
        message: impl Into<String>,
    ) -> Result<(), String> {
        self.record_reconciliation_state(succeeded, false, message)
    }

    pub(crate) fn record_reconciliation_conflict(
        &self,
        message: impl Into<String>,
    ) -> Result<(), String> {
        self.record_reconciliation_state(false, true, message)
    }

    fn record_reconciliation_state(
        &self,
        succeeded: bool,
        requires_confirmation: bool,
        message: impl Into<String>,
    ) -> Result<(), String> {
        let status = VolatileRecoveryStatus {
            session_id: self.session_id.clone(),
            created_at_ms: self.created_at_ms,
            reconciliation_verified: succeeded,
            cleanup_eligible: succeeded,
            requires_confirmation,
            last_result: Some(sanitize_health_text(&message.into(), 768)),
        };
        self.write_manifest_status(&status)?;
        let mut progress = self
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        progress.reconciliation_verified = status.reconciliation_verified;
        progress.cleanup_eligible = status.cleanup_eligible;
        progress.requires_confirmation = status.requires_confirmation;
        progress.last_result = status.last_result;
        Ok(())
    }

    pub(crate) fn export_encrypted_copy<F, G>(
        &self,
        destination: &Path,
        snapshot_database: F,
        snapshot_operations_database: G,
    ) -> Result<PathBuf, String>
    where
        F: FnOnce(&Path, &Path) -> Result<(), String>,
        G: FnOnce(&Path, &Path) -> Result<(), String>,
    {
        if destination.exists() {
            return Err("Recovery export destination already exists.".to_string());
        }
        fs::create_dir(destination).map_err(|error| error.to_string())?;
        let result = (|| {
            set_private_directory_permissions(destination)?;
            let source_database = self.path_for_file("state")?;
            if !source_database.is_file() {
                return Err(
                    "The volatile recovery session has no encrypted state database.".to_string(),
                );
            }
            let destination_database = destination.join("state.sqlite");
            snapshot_database(&source_database, &destination_database)?;
            let source_operations_database = self.root.join("oomu_ops.db");
            if !source_operations_database.is_file() {
                return Err(
                    "The volatile recovery session has no encrypted operations database."
                        .to_string(),
                );
            }
            let destination_operations_database = destination.join("oomu_ops.db");
            snapshot_operations_database(
                &source_operations_database,
                &destination_operations_database,
            )?;

            let status = self.snapshot();
            let manifest = serde_json::to_vec_pretty(&status).map_err(|error| error.to_string())?;
            let manifest_path = destination.join("recovery-status.json");
            fs::write(&manifest_path, manifest).map_err(|error| error.to_string())?;
            set_private_file_permissions(&manifest_path)?;
            enforce_private_tree(destination)
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(destination);
            return Err(error);
        }
        Ok(destination.to_path_buf())
    }

    pub(crate) fn cleanup_after_reconciliation(&self) -> Result<(), String> {
        let progress = self
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !progress.cleanup_eligible || !progress.reconciliation_verified {
            return Err(
                "Volatile storage cannot be deleted until durable reconciliation is verified."
                    .to_string(),
            );
        }
        let expected = VolatileRecoveryStatus {
            session_id: self.session_id.clone(),
            created_at_ms: self.created_at_ms,
            reconciliation_verified: progress.reconciliation_verified,
            cleanup_eligible: progress.cleanup_eligible,
            requires_confirmation: progress.requires_confirmation,
            last_result: progress.last_result.clone(),
        };
        drop(progress);
        let manifest = self.read_manifest_status()?;
        if manifest != expected {
            return Err(
                "Volatile storage cleanup refused because durable manifest evidence does not match the verified in-memory recovery state."
                    .to_string(),
            );
        }
        fs::remove_dir_all(&self.root).map_err(|error| error.to_string())
    }

    fn write_manifest(&self) -> Result<(), String> {
        self.write_manifest_status(&self.snapshot())
    }

    fn write_manifest_status(&self, status: &VolatileRecoveryStatus) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&status).map_err(|error| error.to_string())?;
        let path = self.root.join("recovery-status.json");
        let temporary_path = self.root.join("recovery-status.json.tmp");
        let mut file = fs::File::create(&temporary_path).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        set_private_file_permissions(&temporary_path)?;
        fs::rename(&temporary_path, &path).map_err(|error| error.to_string())?;
        fs::File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| error.to_string())?;
        let persisted = self.read_manifest_status()?;
        if &persisted != status {
            return Err("Volatile recovery manifest verification failed after write.".to_string());
        }
        Ok(())
    }

    fn read_manifest_status(&self) -> Result<VolatileRecoveryStatus, String> {
        let path = self.root.join("recovery-status.json");
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if metadata.len() > 16 * 1024 {
            return Err("Volatile recovery manifest exceeds the bounded status size.".to_string());
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid volatile recovery manifest: {error}"))
    }
}

fn discover_pending_session_in(base: &Path) -> Result<Option<VolatileStoreSession>, String> {
    discover_pending_session_in_excluding(base, None)
}

fn discover_pending_session_in_excluding(
    base: &Path,
    excluded_root: Option<&Path>,
) -> Result<Option<VolatileStoreSession>, String> {
    if !base.exists() {
        return Ok(None);
    }
    set_private_directory_permissions(base)?;
    let mut candidates = Vec::new();
    for entry in fs::read_dir(base).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let root = entry.path();
        if excluded_root.is_some_and(|excluded| excluded == root) {
            continue;
        }
        let manifest_path = root.join("recovery-status.json");
        let state_path = root.join("state.sqlite");
        let has_manifest = manifest_path.is_file();
        let has_state = state_path.is_file();
        if !has_manifest && !has_state {
            continue;
        }
        if has_manifest != has_state {
            return Err(
                "Incomplete volatile recovery session detected; startup refuses to abandon or delete potential recovery data."
                    .to_string(),
            );
        }
        let mut state_header = [0u8; 16];
        let header_read = fs::File::open(&state_path)
            .and_then(|mut file| file.read_exact(&mut state_header))
            .is_ok();
        if header_read && &state_header == b"SQLite format 3\0" {
            return Err("Volatile recovery state is plaintext; startup refuses it.".to_string());
        }
        let manifest_metadata = fs::metadata(&manifest_path).map_err(|error| error.to_string())?;
        if manifest_metadata.len() > 16 * 1024 {
            return Err("Volatile recovery manifest exceeds the bounded status size.".to_string());
        }
        let status: VolatileRecoveryStatus =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(|error| error.to_string())?)
                .map_err(|error| format!("Invalid volatile recovery manifest: {error}"))?;
        let directory_name = root.file_name().and_then(|value| value.to_str());
        if directory_name != Some(status.session_id.as_str())
            || status.session_id.len() != 64
            || !status
                .session_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Volatile recovery manifest identity does not match its directory.".into());
        }
        enforce_private_tree(&root)?;
        candidates.push((status.created_at_ms, root, status));
    }
    candidates.sort_by_key(|(created_at_ms, _, _)| *created_at_ms);
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_, root, status)| VolatileStoreSession {
            session_id: status.session_id,
            root,
            created_at_ms: status.created_at_ms,
            progress: Mutex::new(VolatileRecoveryProgress {
                // The status file is deliberately non-secret renderer metadata, not an
                // authorization artifact. A restart must perform a fresh durable read/write
                // probe before cleanup so a stale or modified manifest can never authorize
                // deletion of the encrypted recovery database.
                reconciliation_verified: false,
                cleanup_eligible: false,
                requires_confirmation: status.requires_confirmation,
                last_result: Some(if status.reconciliation_verified || status.cleanup_eligible {
                    "A previous reconciliation was recorded; durable recovery must be verified again after restart before cleanup."
                        .to_string()
                } else {
                    sanitize_health_text(
                        status
                            .last_result
                            .as_deref()
                            .unwrap_or("Encrypted recovery data requires reconciliation or export."),
                        768,
                    )
                }),
            }),
        }))
}

fn recovery_base_directory() -> PathBuf {
    std::env::temp_dir().join("oomu-degraded-startup")
}

fn validate_store_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("Volatile store names may contain only letters, digits, '-' and '_'.".into());
    }
    Ok(())
}

fn sanitize_health_text(input: &str, max_chars: usize) -> String {
    let redacted = crate::redaction::redact_text(input);
    if redacted.chars().count() <= max_chars {
        return redacted;
    }
    let mut bounded = redacted.chars().take(max_chars).collect::<String>();
    bounded.push('…');
    bounded
}

fn random_hex(byte_count: usize) -> String {
    let mut bytes = vec![0u8; byte_count];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn enforce_private_tree(root: &Path) -> Result<(), String> {
    set_private_directory_permissions(root)?;
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            return Err("Private volatile storage refuses symbolic links.".to_string());
        }
        if file_type.is_dir() {
            enforce_private_tree(&entry.path())?;
        } else if file_type.is_file() {
            set_private_file_permissions(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_recovery_base(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oomu-recovery-{label}-{}-{}",
            std::process::id(),
            random_hex(8)
        ))
    }

    fn write_recovery_fixture(
        base: &Path,
        session_id: &str,
        created_at_ms: i64,
        requires_confirmation: bool,
    ) -> PathBuf {
        fs::create_dir_all(base).unwrap();
        set_private_directory_permissions(base).unwrap();
        let root = base.join(session_id);
        fs::create_dir(&root).unwrap();
        set_private_directory_permissions(&root).unwrap();
        fs::write(root.join("state.sqlite"), b"encrypted-state-placeholder").unwrap();
        let status = VolatileRecoveryStatus {
            session_id: session_id.to_string(),
            created_at_ms,
            reconciliation_verified: false,
            cleanup_eligible: false,
            requires_confirmation,
            last_result: Some("recovery required".to_string()),
        };
        fs::write(
            root.join("recovery-status.json"),
            serde_json::to_vec(&status).unwrap(),
        )
        .unwrap();
        enforce_private_tree(&root).unwrap();
        root
    }

    #[test]
    fn unrelated_recovery_cannot_clear_persistence_failure() {
        let state = DegradedModeState::default();
        state.activate(
            "chatSessionPersistence",
            "durable database unavailable",
            BackingStoreClass::Volatile,
            true,
            "New chats are stored only in this recovery session.",
        );
        state.activate(
            "inference",
            "model unavailable",
            BackingStoreClass::NotApplicable,
            true,
            "Local generation is unavailable.",
        );

        assert!(state.clear_after_verified_recovery(
            "inference",
            BackingStoreClass::NotApplicable,
            "model probe succeeded"
        ));

        let snapshot = state.snapshot();
        assert!(snapshot.active);
        assert!(snapshot.has_volatile_storage);
        assert!(snapshot
            .subsystems
            .iter()
            .any(|entry| entry.subsystem == "chatSessionPersistence" && entry.active));
    }

    #[test]
    fn recovery_pending_identity_does_not_claim_volatile_storage() {
        let state = DegradedModeState::default();
        state.activate(
            "identity",
            "secure identity requires another probe",
            BackingStoreClass::RecoveryPending,
            true,
            "Signing is unavailable.",
        );

        let snapshot = state.snapshot();
        assert!(snapshot.active);
        assert!(!snapshot.has_volatile_storage);
    }

    #[test]
    fn volatile_session_is_random_private_and_cleanup_requires_reconciliation() {
        let base = isolated_recovery_base("private-session");
        let first = VolatileStoreSession::create_in(&base).unwrap();
        let second = VolatileStoreSession::create_in(&base).unwrap();
        assert_ne!(first.session_id, second.session_id);
        assert_eq!(first.session_id.len(), 64);
        assert!(first.cleanup_after_reconciliation().is_err());

        let file = first.path_for_file("state").unwrap();
        fs::write(&file, b"encrypted-placeholder").unwrap();
        first.enforce_private_tree().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(first.root()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        first
            .record_reconciliation(true, "durable read/write probe succeeded")
            .unwrap();
        first.cleanup_after_reconciliation().unwrap();
        second.record_reconciliation(true, "test cleanup").unwrap();
        second.cleanup_after_reconciliation().unwrap();
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn session_manager_advances_across_isolated_retained_sessions() {
        let base = isolated_recovery_base("multi-session");
        let first_root = write_recovery_fixture(&base, &"1".repeat(64), 10, false);
        let second_root = write_recovery_fixture(&base, &"2".repeat(64), 20, false);
        let sessions = VolatileStoreSessionManager::initialize_in(base.clone()).unwrap();

        let first = sessions.current().expect("oldest session is current");
        assert_eq!(first.root(), first_root);
        first.record_reconciliation(true, "verified first").unwrap();
        sessions.cleanup_current_and_advance(&first).unwrap();
        assert!(!first_root.exists());

        let second = sessions.current().expect("second session becomes current");
        assert_eq!(second.root(), second_root);
        second
            .record_reconciliation(true, "verified second")
            .unwrap();
        sessions.cleanup_current_and_advance(&second).unwrap();
        assert!(!second_root.exists());
        assert!(sessions.current().is_none());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn session_manager_preserves_verified_current_when_successor_is_incomplete() {
        let base = isolated_recovery_base("incomplete-successor");
        let first_root = write_recovery_fixture(&base, &"3".repeat(64), 10, false);
        let sessions = VolatileStoreSessionManager::initialize_in(base.clone()).unwrap();
        let first = sessions.current().expect("first session is current");
        first.record_reconciliation(true, "verified first").unwrap();

        let incomplete_root = base.join("4".repeat(64));
        fs::create_dir(&incomplete_root).unwrap();
        fs::write(
            incomplete_root.join("state.sqlite"),
            b"encrypted-state-placeholder",
        )
        .unwrap();
        enforce_private_tree(&incomplete_root).unwrap();

        let error = sessions
            .cleanup_current_and_advance(&first)
            .expect_err("incomplete successor must stop cleanup");
        assert!(error.contains("Incomplete volatile recovery session"));
        assert!(first_root.exists());
        assert!(incomplete_root.exists());
        assert!(Arc::ptr_eq(
            &sessions.current().expect("verified current is retained"),
            &first
        ));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn pending_encrypted_session_is_discovered_after_manager_restart() {
        let base = std::env::temp_dir().join(format!(
            "oomu-recovery-discovery-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&base).unwrap();
        set_private_directory_permissions(&base).unwrap();
        let session_id = "a".repeat(64);
        let root = base.join(&session_id);
        fs::create_dir(&root).unwrap();
        set_private_directory_permissions(&root).unwrap();
        fs::write(root.join("state.sqlite"), b"encrypted-state-placeholder").unwrap();
        let status = VolatileRecoveryStatus {
            session_id: session_id.clone(),
            created_at_ms: 218,
            reconciliation_verified: false,
            cleanup_eligible: false,
            requires_confirmation: true,
            last_result: Some("recovery required".to_string()),
        };
        fs::write(
            root.join("recovery-status.json"),
            serde_json::to_vec(&status).unwrap(),
        )
        .unwrap();
        enforce_private_tree(&root).unwrap();

        let discovered = discover_pending_session_in(&base)
            .unwrap()
            .expect("pending session discovered");
        assert_eq!(discovered.session_id, session_id);
        assert!(!discovered.snapshot().reconciliation_verified);
        assert!(discovered.snapshot().requires_confirmation);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn discovered_manifest_cannot_authorize_cleanup_without_a_fresh_probe() {
        let base = std::env::temp_dir().join(format!(
            "oomu-recovery-untrusted-manifest-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&base).unwrap();
        set_private_directory_permissions(&base).unwrap();
        let session_id = "c".repeat(64);
        let root = base.join(&session_id);
        fs::create_dir(&root).unwrap();
        set_private_directory_permissions(&root).unwrap();
        fs::write(root.join("state.sqlite"), b"encrypted-state-placeholder").unwrap();
        let status = VolatileRecoveryStatus {
            session_id,
            created_at_ms: 220,
            reconciliation_verified: true,
            cleanup_eligible: true,
            requires_confirmation: false,
            last_result: Some("manifest claims cleanup is authorized".to_string()),
        };
        fs::write(
            root.join("recovery-status.json"),
            serde_json::to_vec(&status).unwrap(),
        )
        .unwrap();
        enforce_private_tree(&root).unwrap();

        let discovered = discover_pending_session_in(&base)
            .unwrap()
            .expect("pending session discovered");
        let discovered_status = discovered.snapshot();
        assert!(!discovered_status.reconciliation_verified);
        assert!(!discovered_status.cleanup_eligible);
        assert!(discovered.cleanup_after_reconciliation().is_err());
        assert!(root.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn discovery_fails_closed_for_orphaned_encrypted_state() {
        let base = std::env::temp_dir().join(format!(
            "oomu-recovery-orphaned-state-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&base).unwrap();
        let session_id = "d".repeat(64);
        let root = base.join(session_id);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("state.sqlite"), b"encrypted-state-placeholder").unwrap();

        let error = discover_pending_session_in(&base).unwrap_err();
        assert!(error.contains("Incomplete volatile recovery session"));
        assert!(root.join("state.sqlite").exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn discovery_refuses_plaintext_sqlite_recovery_state() {
        let base = std::env::temp_dir().join(format!(
            "oomu-recovery-plaintext-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&base).unwrap();
        let session_id = "b".repeat(64);
        let root = base.join(&session_id);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("state.sqlite"), b"SQLite format 3\0plaintext").unwrap();
        let status = VolatileRecoveryStatus {
            session_id,
            created_at_ms: 219,
            reconciliation_verified: false,
            cleanup_eligible: false,
            requires_confirmation: false,
            last_result: None,
        };
        fs::write(
            root.join("recovery-status.json"),
            serde_json::to_vec(&status).unwrap(),
        )
        .unwrap();

        let error = discover_pending_session_in(&base).unwrap_err();
        assert!(error.contains("plaintext"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn renderer_health_payload_redacts_paths_secrets_and_bounds_messages() {
        let state = DegradedModeState::default();
        let secret = "super-secret-health-token";
        state.activate(
            "chatSessionPersistence",
            format!(
                "database /Users/alice/private/state.sqlite failed token={secret} {}",
                "x".repeat(2_000)
            ),
            BackingStoreClass::RecoveryPending,
            true,
            "Writes are blocked.",
        );
        let payload = serde_json::to_string(&state.snapshot()).unwrap();
        assert!(!payload.contains("/Users/alice"));
        assert!(!payload.contains(secret));
        assert!(payload.len() < 2_000);
    }
}
