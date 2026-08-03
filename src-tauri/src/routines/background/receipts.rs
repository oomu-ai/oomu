use super::state::BackgroundRow;
use crate::{
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    routines::BackgroundRuntimeReceipt,
};
use rusqlite::params;

const RECEIPT_RETENTION: i64 = 256;

pub(super) fn recent_receipts(
    connection: &rusqlite::Connection,
    limit: i64,
) -> Result<Vec<BackgroundRuntimeReceipt>, String> {
    let mut statement = connection
        .prepare("SELECT receipt_id,event_kind,outcome,runtime_state,requested_enabled,registration_generation,process_id,build_number,build_identity,profile_class,profile_generation,detail_code,subject_id_hash,result_digest,created_at_ms FROM background_runtime_receipts ORDER BY created_at_ms DESC,receipt_id DESC LIMIT ?1")
        .map_err(|error| error.to_string())?;
    let receipts = statement
        .query_map(params![limit.clamp(1, 64)], |row| {
            Ok(BackgroundRuntimeReceipt {
                receipt_id: row.get(0)?,
                kind: row.get(1)?,
                outcome: row.get(2)?,
                runtime_state: row.get(3)?,
                requested_enabled: row.get(4)?,
                registration_generation: row.get(5)?,
                process_id: row.get(6)?,
                build_number: row.get(7)?,
                build_identity: row.get(8)?,
                profile_class: row.get(9)?,
                profile_generation_sha256: sha256_hex(row.get::<_, String>(10)?.as_bytes()),
                detail_code: row.get(11)?,
                subject_id_hash: row.get(12)?,
                result_digest: row.get(13)?,
                created_at_ms: row.get(14)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string());
    receipts
}

pub(super) fn insert_receipt(
    connection: &rusqlite::Connection,
    row: &BackgroundRow,
    event_kind: &'static str,
    outcome: &'static str,
    detail_code: Option<&str>,
    subject_id_hash: Option<&str>,
    result_digest: Option<&str>,
) -> Result<(), String> {
    let receipt_id = format!("background_{}", crate::p0_contracts::TaskId::new());
    connection
        .execute(
            "INSERT INTO background_runtime_receipts (receipt_id,event_kind,outcome,runtime_state,requested_enabled,registration_generation,process_id,build_number,build_identity,profile_class,profile_generation,detail_code,subject_id_hash,result_digest,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                &receipt_id,
                event_kind,
                outcome,
                &row.runtime_state,
                row.requested_enabled,
                row.registration_generation.as_deref(),
                row.process_id,
                row.build_number,
                &row.build_identity,
                &row.profile_class,
                &row.profile_generation,
                detail_code,
                subject_id_hash,
                result_digest,
                unix_time_ms_i64()
            ],
        )
        .map_err(|error| error.to_string())?;
    emit_native_receipt(
        &receipt_id,
        row,
        event_kind,
        outcome,
        detail_code,
        subject_id_hash,
        result_digest,
    );
    connection
        .execute(
            "DELETE FROM background_runtime_receipts WHERE receipt_id IN (SELECT receipt_id FROM background_runtime_receipts ORDER BY created_at_ms DESC,receipt_id DESC LIMIT -1 OFFSET ?1)",
            params![RECEIPT_RETENTION],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn emit_native_receipt(
    receipt_id: &str,
    row: &BackgroundRow,
    event_kind: &str,
    outcome: &str,
    detail_code: Option<&str>,
    subject_id_hash: Option<&str>,
    result_digest: Option<&str>,
) {
    let kind = match event_kind {
        "requested_state_changed" => "background_requested",
        "registration_started" | "registration_verified" | "registration_failed" => {
            "background_registration"
        }
        "heartbeat_verified" => "background_heartbeat",
        "runtime_stopped" => "background_worker_stopped",
        "attention_required" => "background_needs_attention",
        "menu_shown" if row.runtime_state == "on_verified" => "background_verified",
        "menu_hidden" if row.runtime_state == "needs_attention" => "background_needs_attention",
        "shutdown_verified" => "background_stopped",
        "menu_shown" | "menu_hidden" => "background_menu_visibility",
        "window_closed" => "background_window_closed",
        "window_reopened" => "background_window_reopened",
        "quit_requested" => "background_quit_requested",
        "scheduled_postcondition_verified" => "background_postcondition_verified",
        "reconciliation_started" => "background_reconciliation_started",
        "reconciliation_verified" | "reconciliation_failed" => "background_reconciliation",
        _ => "background_runtime_event",
    };
    let registration_generation = row
        .registration_generation
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok());
    let heartbeat_fresh = row
        .heartbeat_expires_at_ms
        .is_some_and(|expires| expires >= unix_time_ms_i64());
    let receipt = serde_json::json!({
        "schema": "oomu.background-runtime.v1",
        "receiptId": receipt_id,
        "kind": kind,
        "event": event_kind,
        "outcome": outcome,
        "state": row.runtime_state,
        "requestedEnabled": row.requested_enabled,
        "registered": row.registration_state == "registered",
        "registrationBackend": row.registration_backend,
        "registrationGeneration": registration_generation,
        "processId": row.process_id,
        "buildIdentity": row.build_identity,
        "buildNumber": row.build_number,
        "profileClass": row.profile_class,
        "profileGenerationSha256": sha256_hex(row.profile_generation.as_bytes()),
        "heartbeatFresh": heartbeat_fresh,
        "menuVisible": row.menu_visible,
        "recoveryAction": (row.runtime_state == "needs_attention").then_some("repair"),
        "detailCode": detail_code,
        "subjectIdHash": subject_id_hash,
        "resultDigest": result_digest,
        "createdAtMs": unix_time_ms_i64(),
    });
    eprintln!("OOMU_BACKGROUND_RECEIPT {receipt}");
}
