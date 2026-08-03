use super::{
    current_identity, ensure_state, load_row, BackgroundRow, RuntimeIdentity, TURNING_GRACE_MS,
};
use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    routines::BackgroundServiceStatus,
};
use rusqlite::params;

use crate::routines::background::receipts::recent_receipts;

pub(in crate::routines::background) fn status(
    engine: &PersistenceEngine,
) -> Result<BackgroundServiceStatus, String> {
    ensure_state(engine)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    status_from_row(&connection, row)
}

pub(in crate::routines::background) fn menu_activation_ready(
    engine: &PersistenceEngine,
) -> Result<bool, String> {
    ensure_state(engine)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let row = load_row(&connection)?;
    Ok(worker_evidence_is_current(
        &row,
        &current_identity(),
        unix_time_ms_i64(),
    ))
}

fn status_from_row(
    connection: &rusqlite::Connection,
    mut row: BackgroundRow,
) -> Result<BackgroundServiceStatus, String> {
    let now = unix_time_ms_i64();
    let next_state = derived_runtime_state(&row, &current_identity(), now);
    if row.runtime_state != next_state {
        connection
            .execute(
                "UPDATE background_service_state SET runtime_state=?1,updated_at_ms=?2 WHERE singleton=1",
                params![next_state, now],
            )
            .map_err(|error| error.to_string())?;
        row = load_row(connection)?;
    }
    let heartbeat_age_ms = row.heartbeat_at_ms.map(|value| now.saturating_sub(value));
    let receipts = recent_receipts(connection, 24)?;
    Ok(BackgroundServiceStatus {
        user_enabled: row.requested_enabled,
        verified_active: row.runtime_state == "on_verified",
        state: row.runtime_state,
        registration_state: row.registration_state,
        registration_backend: row.registration_backend,
        process_state: row.process_state,
        registration_generation: row.registration_generation,
        process_id: row.process_id,
        build_number: row.build_number,
        build_identity: row.build_identity,
        profile_class: row.profile_class,
        profile_generation_sha256: sha256_hex(row.profile_generation.as_bytes()),
        heartbeat_at_ms: row.heartbeat_at_ms,
        heartbeat_age_ms,
        menu_visible: row.menu_visible,
        error_code: row.last_error_code.clone(),
        detail: status_detail(row.last_error_code.as_deref()),
        checked_at_ms: now,
        recent_receipts: receipts,
    })
}

pub(super) fn derived_runtime_state(
    row: &BackgroundRow,
    identity: &RuntimeIdentity,
    now: i64,
) -> String {
    if row.last_error_code.as_deref() == Some("background_menu_evidence_failed") {
        return "needs_attention".to_string();
    }
    if !row.requested_enabled {
        return if row.runtime_state == "turning_off"
            && now.saturating_sub(row.updated_at_ms) <= TURNING_GRACE_MS
            && (row.registration_state != "unregistered" || row.process_state != "absent")
        {
            "turning_off"
        } else if row.registration_state == "unregistered"
            && row.process_state == "absent"
            && !row.menu_visible
        {
            "off"
        } else {
            "needs_attention"
        }
        .to_string();
    }
    let heartbeat_current = worker_evidence_is_current(row, identity, now);
    if row.registration_state == "registered" && heartbeat_current {
        return if row.menu_visible {
            "on_verified"
        } else {
            "turning_on"
        }
        .to_string();
    }
    if row.runtime_state == "turning_on"
        && now.saturating_sub(row.updated_at_ms) <= TURNING_GRACE_MS
    {
        "turning_on".to_string()
    } else {
        "needs_attention".to_string()
    }
}

fn worker_evidence_is_current(row: &BackgroundRow, identity: &RuntimeIdentity, now: i64) -> bool {
    row.requested_enabled
        && row.registration_state == "registered"
        && row.last_error_code.is_none()
        && row
            .heartbeat_expires_at_ms
            .is_some_and(|value| value >= now)
        && row.heartbeat_at_ms.is_some()
        && row.process_id.is_some_and(|process_id| process_id > 0)
        && row.process_state == "running"
        && row.build_number == identity.build_number
        && row.build_identity == identity.build_identity
        && row.profile_class == identity.profile_class
        && !row.profile_generation.trim().is_empty()
        && row.registration_generation.is_some()
}

fn status_detail(code: Option<&str>) -> String {
    match code {
        None => "Background runtime state was checked against native registration and a current worker heartbeat.",
        Some("background_requires_approval") => {
            "macOS approval is required before background work can run."
        }
        Some("background_requires_signed_install") => {
            "A signed installed copy is required before background work can run."
        }
        Some("background_runtime_worker_stopped") => {
            "The background worker stopped and needs to be started again."
        }
        Some("background_registration_lost") => {
            "Background work needs to reconnect before it can continue."
        }
        Some("background_runtime_watchdog_failed" | "background_menu_evidence_failed") => {
            "Background work stopped safely and needs to be started again."
        }
        Some(_) => "Background work needs attention before it can run.",
    }
    .to_string()
}
