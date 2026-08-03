use super::{
    db, BackingStoreClass, DegradedModeState, VolatileRecoveryStatus, VolatileStoreSession,
    VolatileStoreSessionManager,
};
use std::path::PathBuf;

const MAX_STARTUP_RECOVERY_SESSIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StartupRecoveryDrainResult {
    pub recovered_sessions: usize,
    pub fully_drained: bool,
}

fn safe_recovery_error(error: impl ToString) -> String {
    let redacted = crate::redaction::redact_text(&error.to_string());
    if redacted.chars().count() <= 768 {
        redacted
    } else {
        format!("{}…", redacted.chars().take(768).collect::<String>())
    }
}

fn reconcile_discovered_session_without_overwrite(
    persistence: &db::PersistenceEngine,
    session: &VolatileStoreSession,
) -> Result<db::PersistenceRecoveryReport, String> {
    let state_path = session.path_for_file("state")?;
    if !state_path.is_file() {
        return Err("The discovered recovery session has no encrypted state database.".into());
    }
    let _active_store_guard = persistence.lock_writes();
    let recovery_engine =
        db::PersistenceEngine::initialize_volatile_at(state_path).map_err(safe_recovery_error)?;
    recovery_engine
        .reconcile_volatile_store(false)
        .map_err(safe_recovery_error)
}

fn drain_non_conflicting_sessions_at_startup<R, P>(
    degraded_mode: &DegradedModeState,
    sessions: &VolatileStoreSessionManager,
    max_sessions: usize,
    mut reconcile: R,
    mut probe_durable_store: P,
) -> Result<StartupRecoveryDrainResult, String>
where
    R: FnMut(&VolatileStoreSession) -> Result<db::PersistenceRecoveryReport, String>,
    P: FnMut() -> Result<(), String>,
{
    let mut recovered_sessions = 0usize;
    for _ in 0..max_sessions {
        let Some(session) = sessions.current() else {
            degraded_mode.clear_after_verified_recovery(
                "chatSessionPersistence",
                BackingStoreClass::Persistent,
                "Durable storage was verified and every non-conflicting temporary session was recovered.",
            );
            return Ok(StartupRecoveryDrainResult {
                recovered_sessions,
                fully_drained: true,
            });
        };
        let report = reconcile(&session)?;
        if report.requires_confirmation {
            degraded_mode.mark_recovery_pending(
                "chatSessionPersistence",
                "Durable data conflicts were found; explicit overwrite confirmation is required.",
            );
            session.record_reconciliation_conflict(
                "Reconciliation paused because durable records require confirmation.",
            )?;
            return Ok(StartupRecoveryDrainResult {
                recovered_sessions,
                fully_drained: false,
            });
        }
        if report.failed_records != 0 || !report.durable_probe_verified {
            return Err("Recovery verification did not complete successfully.".to_string());
        }

        probe_durable_store()?;
        session.record_reconciliation(
            true,
            format!(
                "Recovered {} record(s), skipped {}, conflicts {}, failures {}.",
                report.recovered_records,
                report.skipped_records,
                report.conflicting_records,
                report.failed_records
            ),
        )?;
        sessions.cleanup_current_and_advance(&session)?;
        recovered_sessions = recovered_sessions.saturating_add(1);
    }

    if sessions.current().is_none() {
        degraded_mode.clear_after_verified_recovery(
            "chatSessionPersistence",
            BackingStoreClass::Persistent,
            "Durable storage was verified and every non-conflicting temporary session was recovered.",
        );
        return Ok(StartupRecoveryDrainResult {
            recovered_sessions,
            fully_drained: true,
        });
    }

    Err("The bounded startup recovery limit was reached; remaining encrypted recovery sessions were preserved.".to_string())
}

/// Automatically drains retained sessions only while each can be
/// reconciled without replacing durable records. The first conflict,
/// incomplete session, or error stops the bounded pass and leaves that
/// session intact for the user's explicit choice.
pub(super) fn reconcile_non_conflicting_sessions_at_startup(
    persistence: &db::PersistenceEngine,
    degraded_mode: &DegradedModeState,
    sessions: &VolatileStoreSessionManager,
) -> Result<StartupRecoveryDrainResult, String> {
    if persistence.storage_class() != BackingStoreClass::Persistent {
        return Ok(StartupRecoveryDrainResult {
            recovered_sessions: 0,
            fully_drained: false,
        });
    }

    drain_non_conflicting_sessions_at_startup(
        degraded_mode,
        sessions,
        MAX_STARTUP_RECOVERY_SESSIONS,
        |session| reconcile_discovered_session_without_overwrite(persistence, session),
        || {
            persistence
                .probe_active_durable_store()
                .map_err(safe_recovery_error)
        },
    )
}

#[tauri::command]
pub fn get_persistence_recovery_status(
    sessions: tauri::State<'_, VolatileStoreSessionManager>,
) -> Option<VolatileRecoveryStatus> {
    sessions.current().map(|session| session.snapshot())
}

#[tauri::command]
pub fn export_volatile_persistence(
    destination: String,
    persistence: tauri::State<'_, db::PersistenceEngine>,
    sessions: tauri::State<'_, VolatileStoreSessionManager>,
) -> Result<String, String> {
    let session = sessions
        .current()
        .ok_or_else(|| "No volatile persistence session is active.".to_string())?;
    let destination = PathBuf::from(destination.trim());
    if !destination.is_absolute() {
        return Err("Recovery export destination must be an absolute path.".to_string());
    }
    session
        .export_encrypted_copy(
            &destination,
            |source, target| persistence.export_encrypted_snapshot(source, target),
            |source, target| persistence.export_encrypted_operations_snapshot(source, target),
        )
        .map_err(safe_recovery_error)
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn choose_volatile_persistence_export(
    persistence: tauri::State<'_, db::PersistenceEngine>,
    sessions: tauri::State<'_, VolatileStoreSessionManager>,
) -> Result<Option<String>, String> {
    let session = sessions
        .current()
        .ok_or_else(|| "No volatile persistence session is active.".to_string())?;
    let Some(parent) = rfd::FileDialog::new()
        .set_title("Export Encrypted OOMU Recovery Session")
        .pick_folder()
    else {
        return Ok(None);
    };
    let status = session.snapshot();
    let destination = parent.join(format!("oomu-recovery-{}", status.session_id));
    session
        .export_encrypted_copy(
            &destination,
            |source, target| persistence.export_encrypted_snapshot(source, target),
            |source, target| persistence.export_encrypted_operations_snapshot(source, target),
        )
        .map_err(safe_recovery_error)
        .map(|path| Some(path.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn reconcile_volatile_persistence(
    confirm_overwrite: bool,
    persistence: tauri::State<'_, db::PersistenceEngine>,
    degraded_mode: tauri::State<'_, DegradedModeState>,
    sessions: tauri::State<'_, VolatileStoreSessionManager>,
) -> Result<db::PersistenceRecoveryReport, String> {
    let session = sessions
        .current()
        .ok_or_else(|| "No volatile persistence session is active.".to_string())?;
    let report = if persistence.storage_class() == BackingStoreClass::Persistent {
        if confirm_overwrite {
            let state_path = session.path_for_file("state")?;
            if !state_path.is_file() {
                return Err(
                    "The discovered recovery session has no encrypted state database.".into(),
                );
            }
            let _active_store_guard = persistence.lock_writes();
            let recovery_engine = db::PersistenceEngine::initialize_volatile_at(state_path)
                .map_err(safe_recovery_error)?;
            recovery_engine
                .reconcile_volatile_store(true)
                .map_err(safe_recovery_error)?
        } else {
            reconcile_discovered_session_without_overwrite(&persistence, &session)?
        }
    } else {
        persistence
            .reconcile_volatile_store(confirm_overwrite)
            .map_err(safe_recovery_error)?
    };
    if report.requires_confirmation {
        degraded_mode.mark_recovery_pending(
            "chatSessionPersistence",
            "Durable data conflicts were found; explicit overwrite confirmation is required.",
        );
        session.record_reconciliation_conflict(
            "Reconciliation paused because durable records require confirmation.",
        )?;
    } else {
        persistence
            .probe_active_durable_store()
            .map_err(safe_recovery_error)?;
        degraded_mode.mark_reconciled_cleanup_pending(
            "chatSessionPersistence",
            "Schema, key, integrity, capacity, durable write, and durable read probes succeeded after reconciliation.",
        );
        session.record_reconciliation(
            true,
            format!(
                "Recovered {} record(s), skipped {}, conflicts {}, failures {}.",
                report.recovered_records,
                report.skipped_records,
                report.conflicting_records,
                report.failed_records
            ),
        )?;
    }
    Ok(report)
}

#[tauri::command]
pub fn cleanup_reconciled_volatile_persistence(
    persistence: tauri::State<'_, db::PersistenceEngine>,
    degraded_mode: tauri::State<'_, DegradedModeState>,
    sessions: tauri::State<'_, VolatileStoreSessionManager>,
) -> Result<(), String> {
    persistence
        .probe_active_durable_store()
        .map_err(safe_recovery_error)?;
    let session = sessions
        .current()
        .ok_or_else(|| "No volatile persistence session is active.".to_string())?;
    session
        .cleanup_after_reconciliation()
        .map_err(safe_recovery_error)?;
    sessions.forget_cleaned().map_err(safe_recovery_error)?;
    if sessions.current().is_some() {
        degraded_mode.activate(
            "chatSessionPersistence",
            "Another private encrypted recovery session remains and requires reconciliation or export.",
            BackingStoreClass::RecoveryPending,
            true,
            "Earlier chat/session writes remain in encrypted recovery storage until explicitly reconciled and cleaned up.",
        );
    } else {
        degraded_mode.clear_after_verified_recovery(
            "chatSessionPersistence",
            BackingStoreClass::Persistent,
            "Durable reconciliation remained verified and private volatile artifacts were explicitly cleaned up.",
        );
    }
    Ok(())
}

#[tauri::command]
pub fn restore_persistence_migration_backup(
    backup_path: String,
    persistence: tauri::State<'_, db::PersistenceEngine>,
    degraded_mode: tauri::State<'_, DegradedModeState>,
) -> Result<(), String> {
    persistence
        .restore_migration_backup(PathBuf::from(backup_path.trim()).as_path())
        .map_err(safe_recovery_error)?;
    degraded_mode.mark_recovery_pending(
        "chatSessionPersistence",
        "Verified migration backup restored; restart is required to re-run and verify pending migrations.",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn isolated_base(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oomu-startup-recovery-{label}-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn write_session(
        base: &Path,
        session_id: &str,
        created_at_ms: i64,
        requires_confirmation: bool,
    ) -> PathBuf {
        fs::create_dir_all(base).unwrap();
        let root = base.join(session_id);
        fs::create_dir(&root).unwrap();
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
        root
    }

    fn verified_report() -> db::PersistenceRecoveryReport {
        db::PersistenceRecoveryReport {
            recovered_records: 0,
            skipped_records: 0,
            conflicting_records: 0,
            failed_records: 0,
            durable_probe_verified: true,
            requires_confirmation: false,
            backup_created: false,
        }
    }

    fn active_recovery_state() -> DegradedModeState {
        let state = DegradedModeState::default();
        state.activate(
            "chatSessionPersistence",
            "retained encrypted recovery sessions require verification",
            BackingStoreClass::RecoveryPending,
            true,
            "Earlier writes remain in encrypted recovery storage.",
        );
        state
    }

    #[test]
    fn startup_drain_recovers_every_non_conflicting_session_before_clearing_health() {
        let base = isolated_base("all-clear");
        let first_root = write_session(&base, &"a".repeat(64), 10, false);
        let second_root = write_session(&base, &"b".repeat(64), 20, false);
        let sessions = VolatileStoreSessionManager::initialize_in(base.clone()).unwrap();
        let degraded_mode = active_recovery_state();
        let mut reconciled = Vec::new();
        let mut probes = 0usize;

        let result = drain_non_conflicting_sessions_at_startup(
            &degraded_mode,
            &sessions,
            8,
            |session| {
                reconciled.push(session.snapshot().session_id);
                Ok(verified_report())
            },
            || {
                probes += 1;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(result.recovered_sessions, 2);
        assert!(result.fully_drained);
        assert_eq!(reconciled, vec!["a".repeat(64), "b".repeat(64)]);
        assert_eq!(probes, 2);
        assert!(!first_root.exists());
        assert!(!second_root.exists());
        assert!(sessions.current().is_none());
        assert!(!degraded_mode.snapshot().active);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn startup_drain_stops_at_conflict_without_deleting_or_overwriting_it() {
        let base = isolated_base("conflict");
        let conflict_root = write_session(&base, &"c".repeat(64), 10, false);
        let sessions = VolatileStoreSessionManager::initialize_in(base.clone()).unwrap();
        let degraded_mode = active_recovery_state();

        let result = drain_non_conflicting_sessions_at_startup(
            &degraded_mode,
            &sessions,
            8,
            |_session| {
                let mut report = verified_report();
                report.conflicting_records = 1;
                report.requires_confirmation = true;
                Ok(report)
            },
            || panic!("a conflict must stop before the durable completion probe"),
        )
        .unwrap();

        assert_eq!(result.recovered_sessions, 0);
        assert!(!result.fully_drained);
        assert!(conflict_root.exists());
        assert!(
            sessions
                .current()
                .expect("conflicting session remains current")
                .snapshot()
                .requires_confirmation
        );
        assert!(degraded_mode.snapshot().active);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn startup_drain_rechecks_a_previously_reported_conflict() {
        let base = isolated_base("retained-conflict");
        let first_root = write_session(&base, &"d".repeat(64), 10, false);
        let conflict_root = write_session(&base, &"e".repeat(64), 20, true);
        let sessions = VolatileStoreSessionManager::initialize_in(base.clone()).unwrap();
        let degraded_mode = active_recovery_state();
        let mut calls = 0usize;

        let result = drain_non_conflicting_sessions_at_startup(
            &degraded_mode,
            &sessions,
            8,
            |_session| {
                calls += 1;
                Ok(verified_report())
            },
            || Ok(()),
        )
        .unwrap();

        assert_eq!(calls, 2);
        assert_eq!(result.recovered_sessions, 2);
        assert!(result.fully_drained);
        assert!(!first_root.exists());
        assert!(!conflict_root.exists());
        assert!(sessions.current().is_none());
        assert!(!degraded_mode.snapshot().active);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn startup_drain_preserves_current_and_incomplete_successor_on_discovery_error() {
        let base = isolated_base("incomplete");
        let first_root = write_session(&base, &"f".repeat(64), 10, false);
        let sessions = VolatileStoreSessionManager::initialize_in(base.clone()).unwrap();
        let degraded_mode = active_recovery_state();
        let incomplete_root = base.join("0".repeat(64));
        fs::create_dir(&incomplete_root).unwrap();
        fs::write(
            incomplete_root.join("state.sqlite"),
            b"encrypted-state-placeholder",
        )
        .unwrap();

        let error = drain_non_conflicting_sessions_at_startup(
            &degraded_mode,
            &sessions,
            8,
            |_session| Ok(verified_report()),
            || Ok(()),
        )
        .expect_err("incomplete successor must stop the drain");

        assert!(error.contains("Incomplete volatile recovery session"));
        assert!(first_root.exists());
        assert!(incomplete_root.exists());
        assert!(sessions.current().is_some());
        assert!(degraded_mode.snapshot().active);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn startup_drain_is_bounded_and_preserves_sessions_beyond_the_limit() {
        let base = isolated_base("bounded");
        let first_root = write_session(&base, &"1".repeat(64), 10, false);
        let second_root = write_session(&base, &"2".repeat(64), 20, false);
        let sessions = VolatileStoreSessionManager::initialize_in(base.clone()).unwrap();
        let degraded_mode = active_recovery_state();

        let error = drain_non_conflicting_sessions_at_startup(
            &degraded_mode,
            &sessions,
            1,
            |_session| Ok(verified_report()),
            || Ok(()),
        )
        .expect_err("a retained session beyond the bound must pause startup recovery");

        assert!(error.contains("bounded startup recovery limit"));
        assert!(!first_root.exists());
        assert!(second_root.exists());
        assert_eq!(
            sessions.current().unwrap().snapshot().session_id,
            "2".repeat(64)
        );
        assert!(degraded_mode.snapshot().active);
        let _ = fs::remove_dir_all(base);
    }
}
