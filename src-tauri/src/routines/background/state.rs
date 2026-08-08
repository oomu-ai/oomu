mod status;
#[cfg(test)]
mod tests;

use crate::{db::PersistenceEngine, foundation::clock::unix_time_ms_i64};
use rusqlite::params;
use std::sync::OnceLock;

use super::receipts::insert_receipt;
use status::derived_runtime_state;

pub(super) use status::{menu_activation_ready, status};

pub(super) const HEARTBEAT_VALID_FOR: std::time::Duration = std::time::Duration::from_secs(15);
pub(super) const HEARTBEAT_VALID_FOR_MS: i64 = HEARTBEAT_VALID_FOR.as_millis() as i64;
const TURNING_GRACE_MS: i64 = 8_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RuntimeIdentity {
    pub build_number: i64,
    pub build_identity: String,
    pub profile_class: String,
}

#[derive(Clone, Debug)]
pub(super) struct BackgroundRow {
    pub requested_enabled: bool,
    pub runtime_state: String,
    pub registration_state: String,
    pub registration_backend: String,
    pub registration_generation: Option<String>,
    pub process_state: String,
    pub process_id: Option<i64>,
    pub build_number: i64,
    pub build_identity: String,
    pub profile_class: String,
    pub profile_generation: String,
    pub heartbeat_at_ms: Option<i64>,
    pub heartbeat_expires_at_ms: Option<i64>,
    pub menu_visible: bool,
    pub last_error_code: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RegistrationObservation {
    Registered,
    Unregistered,
    RequiresApproval,
    Unavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WatchdogObservation {
    Continue,
    Stop,
    Expired,
}

impl RegistrationObservation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::Unregistered => "unregistered",
            Self::RequiresApproval => "requires_approval",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
        }
    }
}

pub(super) fn current_identity() -> RuntimeIdentity {
    static IDENTITY: OnceLock<RuntimeIdentity> = OnceLock::new();
    IDENTITY
        .get_or_init(|| current_identity_from_process(&crate::macos_process_identity::current()))
        .clone()
}

pub(super) fn current_identity_from_process(
    process: &crate::macos_process_identity::MacosProcessIdentityEvidence,
) -> RuntimeIdentity {
    let profile_class = crate::runtime_profile::current_class(process)
        .map(|class| class.as_str())
        .unwrap_or("unverified");
    RuntimeIdentity {
        build_number: i64::try_from(process.build_number).unwrap_or(i64::MAX),
        build_identity: crate::runtime_profile::identity_component(process).to_string(),
        profile_class: profile_class.to_string(),
    }
}

pub(super) fn ensure_state(engine: &PersistenceEngine) -> Result<BackgroundRow, String> {
    let now = unix_time_ms_i64();
    let identity = current_identity();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT OR IGNORE INTO background_service_state (singleton,user_enabled,service_status,last_error_code,updated_at_ms,requested_enabled,runtime_state,registration_state,registration_backend,process_state,build_number,build_identity,profile_class,profile_generation,menu_visible) VALUES (1,0,'paused',NULL,?1,0,'off','unregistered','unknown','absent',?2,?3,?4,?5,0)",
            params![
                now,
                identity.build_number,
                identity.build_identity,
                identity.profile_class,
                crate::p0_contracts::TaskId::new().to_string()
            ],
        )
        .map_err(|error| error.to_string())?;
    let profile_generation: String = connection
        .query_row(
            "SELECT profile_generation FROM background_service_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if profile_generation.trim().is_empty() {
        connection
            .execute(
                "UPDATE background_service_state SET profile_generation=?1,updated_at_ms=?2 WHERE singleton=1 AND profile_generation=''",
                params![crate::p0_contracts::TaskId::new().to_string(), now],
            )
            .map_err(|error| error.to_string())?;
    }
    load_row(&connection)
}

pub(super) fn set_registration_backend(
    engine: &PersistenceEngine,
    backend: &str,
) -> Result<(), String> {
    ensure_state(engine)?;
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE background_service_state SET registration_backend=?1 WHERE singleton=1",
            params![backend],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(super) fn begin_transition(
    engine: &PersistenceEngine,
    requested_enabled: bool,
    receipt_kind: &'static str,
) -> Result<BackgroundRow, String> {
    let previous = ensure_state(engine)?;
    let identity = current_identity();
    let now = unix_time_ms_i64();
    let generation = requested_enabled
        .then(|| {
            previous
                .registration_generation
                .as_deref()
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0)
                .saturating_add(1)
                .to_string()
        })
        .or(previous.registration_generation.clone());
    let runtime_state = if requested_enabled {
        "turning_on"
    } else {
        "turning_off"
    };
    let registration_state = if requested_enabled {
        "registering"
    } else {
        previous.registration_state.as_str()
    };
    let process_state = if requested_enabled {
        "starting"
    } else {
        "stopping"
    };
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE background_service_state SET user_enabled=?1,requested_enabled=?1,runtime_state=?2,registration_state=?3,registration_generation=?4,process_state=?5,process_id=NULL,build_number=?6,build_identity=?7,profile_class=?8,heartbeat_at_ms=NULL,heartbeat_expires_at_ms=NULL,last_error_code=NULL,updated_at_ms=?9 WHERE singleton=1",
            params![
                requested_enabled,
                runtime_state,
                registration_state,
                generation,
                process_state,
                identity.build_number,
                identity.build_identity,
                identity.profile_class,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    insert_receipt(&connection, &row, receipt_kind, "started", None, None, None)?;
    Ok(row)
}

pub(super) fn observe_registration(
    engine: &PersistenceEngine,
    observation: RegistrationObservation,
    error_code: Option<&str>,
) -> Result<BackgroundRow, String> {
    let now = unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let previous = load_row(&connection)?;
    let next_error = error_code.map(str::to_string).or_else(|| {
        let registration_error = previous.last_error_code.as_deref().is_some_and(|code| {
            code.starts_with("background_registration_")
                || code.starts_with("background_unregistration_")
                || matches!(
                    code,
                    "background_requires_approval"
                        | "background_requires_signed_install"
                        | "smappservice_unavailable"
                )
        });
        (!registration_error)
            .then(|| previous.last_error_code.clone())
            .flatten()
    });
    if previous.registration_state != observation.as_str() || previous.last_error_code != next_error
    {
        connection
            .execute(
                "UPDATE background_service_state SET registration_state=?1,service_status=?2,last_error_code=?3,updated_at_ms=?4 WHERE singleton=1",
                params![
                    observation.as_str(),
                    legacy_service_status(observation),
                    next_error,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    let mut row = load_row(&connection)?;
    let next_state = derived_runtime_state(&row, &current_identity(), now);
    if row.runtime_state != next_state {
        connection
            .execute(
                "UPDATE background_service_state SET runtime_state=?1,updated_at_ms=?2 WHERE singleton=1",
                params![next_state, now],
            )
            .map_err(|error| error.to_string())?;
        row = load_row(&connection)?;
    }
    Ok(row)
}

fn heartbeat_matches_active_generation(
    row: &BackgroundRow,
    generation: &str,
    profile_generation: &str,
    build_number: i64,
    build_identity: &str,
    profile_class: &str,
) -> bool {
    row.requested_enabled
        && row.registration_state == "registered"
        && matches!(row.runtime_state.as_str(), "turning_on" | "on_verified")
        && row.last_error_code.is_none()
        && row.registration_generation.as_deref() == Some(generation)
        && row.profile_generation == profile_generation
        && row.build_number == build_number
        && row.build_identity == build_identity
        && row.profile_class == profile_class
}

pub(super) fn registration_receipt(
    engine: &PersistenceEngine,
    verified: bool,
    detail_code: Option<&str>,
) -> Result<(), String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    insert_receipt(
        &connection,
        &row,
        if verified {
            "registration_verified"
        } else {
            "registration_failed"
        },
        if verified { "verified" } else { "attention" },
        detail_code,
        None,
        None,
    )
}

pub(super) fn registration_started(engine: &PersistenceEngine) -> Result<(), String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    insert_receipt(
        &connection,
        &row,
        "registration_started",
        "started",
        None,
        None,
        None,
    )
}

#[cfg(test)]
pub(super) fn record_heartbeat(
    engine: &PersistenceEngine,
    generation: &str,
    profile_generation: &str,
    build_number: i64,
    build_identity: &str,
    profile_class: &str,
    process_id: i64,
) -> Result<BackgroundRow, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    record_heartbeat_on_connection(
        &connection,
        generation,
        profile_generation,
        build_number,
        build_identity,
        profile_class,
        process_id,
    )
}

pub(super) fn record_heartbeat_on_connection(
    connection: &rusqlite::Connection,
    generation: &str,
    profile_generation: &str,
    build_number: i64,
    build_identity: &str,
    profile_class: &str,
    process_id: i64,
) -> Result<BackgroundRow, String> {
    let now = unix_time_ms_i64();
    let previous = load_row(&connection)?;
    if !heartbeat_matches_active_generation(
        &previous,
        generation,
        profile_generation,
        build_number,
        build_identity,
        profile_class,
    ) {
        return Ok(previous);
    }
    let changed = connection
        .execute(
            "UPDATE background_service_state SET process_state='running',process_id=?1,heartbeat_at_ms=?2,heartbeat_expires_at_ms=?3,last_error_code=NULL,updated_at_ms=?2 WHERE singleton=1 AND requested_enabled=1 AND registration_state='registered' AND runtime_state IN ('turning_on','on_verified') AND last_error_code IS NULL AND registration_generation=?4 AND profile_generation=?5 AND build_number=?6 AND build_identity=?7 AND profile_class=?8",
            params![
                process_id,
                now,
                now + HEARTBEAT_VALID_FOR_MS,
                generation,
                profile_generation,
                build_number,
                build_identity,
                profile_class,
            ],
        )
        .map_err(|error| error.to_string())?;
    let mut row = load_row(&connection)?;
    if changed == 1 {
        let next_state = derived_runtime_state(&row, &current_identity(), now);
        connection
            .execute(
                "UPDATE background_service_state SET runtime_state=?1,updated_at_ms=?2 WHERE singleton=1",
                params![next_state, now],
            )
            .map_err(|error| error.to_string())?;
        row = load_row(&connection)?;
        if previous.runtime_state != "on_verified" || previous.process_id != Some(process_id) {
            insert_receipt(
                &connection,
                &row,
                "heartbeat_verified",
                "verified",
                None,
                None,
                None,
            )?;
        }
    }
    Ok(row)
}

pub(super) fn record_worker_stopped(
    engine: &PersistenceEngine,
    generation: &str,
    intentional: bool,
) -> Result<BackgroundRow, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    record_worker_stopped_on_connection(&connection, generation, intentional)
}

pub(super) fn record_worker_stopped_on_connection(
    connection: &rusqlite::Connection,
    generation: &str,
    intentional: bool,
) -> Result<BackgroundRow, String> {
    let now = unix_time_ms_i64();
    let current = load_row(&connection)?;
    if current.registration_generation.as_deref() != Some(generation) {
        return Ok(current);
    }
    let preserve_attention = current.runtime_state == "needs_attention";
    let next_state = if preserve_attention {
        "needs_attention"
    } else if current.requested_enabled && !intentional {
        "needs_attention"
    } else if current.requested_enabled {
        "turning_on"
    } else {
        "off"
    };
    let error = if preserve_attention {
        current.last_error_code.as_deref()
    } else {
        (current.requested_enabled && !intentional).then_some("background_runtime_worker_stopped")
    };
    let changed = connection
        .execute(
            "UPDATE background_service_state SET runtime_state=?1,process_state='absent',process_id=NULL,heartbeat_at_ms=NULL,heartbeat_expires_at_ms=NULL,last_error_code=?2,updated_at_ms=?3 WHERE singleton=1 AND registration_generation=?4",
            params![next_state, error, now, generation],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return load_row(&connection);
    }
    let row = load_row(&connection)?;
    insert_receipt(
        &connection,
        &row,
        "runtime_stopped",
        if error.is_some() {
            "attention"
        } else {
            "completed"
        },
        error,
        None,
        None,
    )?;
    Ok(row)
}

pub(super) fn expire_stale_heartbeat(
    engine: &PersistenceEngine,
    generation: &str,
) -> Result<WatchdogObservation, String> {
    let now = unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let current = load_row(&connection)?;
    if !current.requested_enabled || current.registration_generation.as_deref() != Some(generation)
    {
        return Ok(WatchdogObservation::Stop);
    }
    let changed = connection
        .execute(
            "UPDATE background_service_state
             SET runtime_state='needs_attention',process_state='absent',process_id=NULL,
                 heartbeat_at_ms=NULL,heartbeat_expires_at_ms=NULL,
                 last_error_code='background_runtime_heartbeat_expired',updated_at_ms=?1
             WHERE singleton=1 AND requested_enabled=1 AND registration_generation=?2
               AND (runtime_state!='needs_attention' OR last_error_code IS NULL
                    OR last_error_code!='background_runtime_heartbeat_expired')
               AND ((heartbeat_expires_at_ms IS NOT NULL AND heartbeat_expires_at_ms<?1)
                    OR (heartbeat_expires_at_ms IS NULL AND updated_at_ms<=?3))",
            params![now, generation, now - HEARTBEAT_VALID_FOR_MS],
        )
        .map_err(|error| error.to_string())?;
    if changed == 1 {
        let row = load_row(&connection)?;
        insert_receipt(
            &connection,
            &row,
            "attention_required",
            "attention",
            Some("background_runtime_heartbeat_expired"),
            None,
            None,
        )?;
        return Ok(WatchdogObservation::Expired);
    }
    let current = load_row(&connection)?;
    if current.runtime_state == "needs_attention" {
        return Ok(WatchdogObservation::Stop);
    }
    Ok(WatchdogObservation::Continue)
}

pub(super) fn record_watchdog_failure(
    engine: &PersistenceEngine,
    generation: &str,
) -> Result<BackgroundRow, String> {
    let now = unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let changed = connection
        .execute(
            "UPDATE background_service_state SET runtime_state='needs_attention',process_state='absent',process_id=NULL,heartbeat_at_ms=NULL,heartbeat_expires_at_ms=NULL,last_error_code='background_runtime_watchdog_failed',updated_at_ms=?1 WHERE singleton=1 AND requested_enabled=1 AND registration_generation=?2",
            params![now, generation],
        )
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    if changed == 1 {
        insert_receipt(
            &connection,
            &row,
            "attention_required",
            "attention",
            Some("background_runtime_watchdog_failed"),
            None,
            None,
        )?;
    }
    Ok(row)
}

pub(super) fn finish_disabled(engine: &PersistenceEngine) -> Result<BackgroundRow, String> {
    let now = unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE background_service_state SET user_enabled=0,requested_enabled=0,runtime_state='off',registration_state='unregistered',service_status='paused',process_state='absent',process_id=NULL,heartbeat_at_ms=NULL,heartbeat_expires_at_ms=NULL,last_error_code=NULL,updated_at_ms=?1 WHERE singleton=1",
            params![now],
        )
        .map_err(|error| error.to_string())?;
    load_row(&connection)
}

pub(super) fn set_attention(
    engine: &PersistenceEngine,
    code: &str,
) -> Result<BackgroundRow, String> {
    let now = unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let previous = load_row(&connection)?;
    if previous.runtime_state == "needs_attention"
        && previous.last_error_code.as_deref() == Some(code)
    {
        return Ok(previous);
    }
    connection
        .execute(
            "UPDATE background_service_state SET runtime_state='needs_attention',last_error_code=?1,updated_at_ms=?2 WHERE singleton=1",
            params![code, now],
        )
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    insert_receipt(
        &connection,
        &row,
        "attention_required",
        "attention",
        Some(code),
        None,
        None,
    )?;
    Ok(row)
}

pub(super) fn record_menu_visibility(
    engine: &PersistenceEngine,
    visible: bool,
) -> Result<(), String> {
    ensure_state(engine)?;
    let now = unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let previous: bool = connection
        .query_row(
            "SELECT menu_visible FROM background_service_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if previous != visible {
        connection
            .execute(
                "UPDATE background_service_state SET menu_visible=?1,updated_at_ms=?2 WHERE singleton=1",
                params![visible, now],
            )
            .map_err(|error| error.to_string())?;
    }
    let mut row = load_row(&connection)?;
    let next_state = derived_runtime_state(&row, &current_identity(), now);
    if row.runtime_state != next_state {
        connection
            .execute(
                "UPDATE background_service_state SET runtime_state=?1,updated_at_ms=?2 WHERE singleton=1",
                params![next_state, now],
            )
            .map_err(|error| error.to_string())?;
        row = load_row(&connection)?;
    }
    if previous != visible {
        insert_receipt(
            &connection,
            &row,
            if visible { "menu_shown" } else { "menu_hidden" },
            "verified",
            None,
            None,
            None,
        )?;
    }
    drop(connection);
    if visible && row.runtime_state == "on_verified" {
        record_reconciliation(engine, true, None)?;
    }
    Ok(())
}

pub(super) fn record_runtime_event(
    engine: &PersistenceEngine,
    event_kind: &'static str,
) -> Result<(), String> {
    ensure_state(engine)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    insert_receipt(&connection, &row, event_kind, "completed", None, None, None)
}

pub(super) fn record_reconciliation(
    engine: &PersistenceEngine,
    verified: bool,
    detail_code: Option<&str>,
) -> Result<(), String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    let latest_reconciliation_event: String = connection
        .query_row(
            "SELECT COALESCE((SELECT event_kind FROM background_runtime_receipts WHERE registration_generation IS ?1 AND event_kind IN ('reconciliation_started','reconciliation_verified','reconciliation_failed') ORDER BY created_at_ms DESC,receipt_id DESC LIMIT 1),'')",
            params![row.registration_generation.as_deref()],
            |result| result.get(0),
        )
        .map_err(|error| error.to_string())?;
    if latest_reconciliation_event != "reconciliation_started" {
        return Ok(());
    }
    insert_receipt(
        &connection,
        &row,
        if verified {
            "reconciliation_verified"
        } else {
            "reconciliation_failed"
        },
        if verified { "verified" } else { "attention" },
        detail_code,
        None,
        None,
    )
}

pub(super) fn generation(engine: &PersistenceEngine) -> Result<(String, String), String> {
    let row = ensure_state(engine)?;
    let registration = row
        .registration_generation
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "background_registration_generation_missing".to_string())?;
    Ok((registration, row.profile_generation))
}

pub(super) fn requested(engine: &PersistenceEngine) -> Result<bool, String> {
    Ok(ensure_state(engine)?.requested_enabled)
}

pub(super) fn registration_backend(engine: &PersistenceEngine) -> Result<String, String> {
    Ok(ensure_state(engine)?.registration_backend)
}

pub(super) fn load_row(connection: &rusqlite::Connection) -> Result<BackgroundRow, String> {
    connection
        .query_row(
            "SELECT requested_enabled,runtime_state,registration_state,registration_backend,registration_generation,process_state,process_id,build_number,build_identity,profile_class,profile_generation,heartbeat_at_ms,heartbeat_expires_at_ms,menu_visible,last_error_code,updated_at_ms FROM background_service_state WHERE singleton=1",
            [],
            |row| {
                Ok(BackgroundRow {
                    requested_enabled: row.get(0)?,
                    runtime_state: row.get(1)?,
                    registration_state: row.get(2)?,
                    registration_backend: row.get(3)?,
                    registration_generation: row.get(4)?,
                    process_state: row.get(5)?,
                    process_id: row.get(6)?,
                    build_number: row.get(7)?,
                    build_identity: row.get(8)?,
                    profile_class: row.get(9)?,
                    profile_generation: row.get(10)?,
                    heartbeat_at_ms: row.get(11)?,
                    heartbeat_expires_at_ms: row.get(12)?,
                    menu_visible: row.get(13)?,
                    last_error_code: row.get(14)?,
                    updated_at_ms: row.get(15)?,
                })
            },
        )
        .map_err(|error| error.to_string())
}

fn legacy_service_status(observation: RegistrationObservation) -> &'static str {
    match observation {
        RegistrationObservation::Registered => "active",
        RegistrationObservation::Unregistered => "paused",
        RegistrationObservation::RequiresApproval => "requires_approval",
        RegistrationObservation::Unavailable => "unavailable",
        RegistrationObservation::Failed => "degraded",
    }
}
