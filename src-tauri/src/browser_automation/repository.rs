use super::{BrowserDownloadView, BrowserSession, BrowserSessionView};
use crate::db::PersistenceEngine;
use rusqlite::{params, OptionalExtension};

pub(super) fn insert_session(
    engine: &PersistenceEngine,
    session: &BrowserSession,
) -> Result<(), String> {
    engine.require_durable_store("start guarded browser automation")?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    engine.open_connection().map_err(|error| error.to_string())?.execute(
        "INSERT INTO browser_automation_sessions (session_id, task_run_id, project_id, canonical_origin, destination_binding, document_generation, state, current_step, last_snapshot_at_ms, created_at_ms, updated_at_ms) VALUES (?1,?2,?3,?4,?5,0,?6,'',NULL,?7,?7)",
        params![session.session_id, session.task_run_id, session.project_id, session.canonical_origin, session.destination_binding, session.state.persisted(), now],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn update_session(
    engine: &PersistenceEngine,
    session: &BrowserSession,
) -> Result<(), String> {
    let changed = engine.open_connection().map_err(|error| error.to_string())?.execute(
        "UPDATE browser_automation_sessions SET document_generation=?2, state=?3, current_step=?4, last_snapshot_at_ms=?5, updated_at_ms=?6 WHERE session_id=?1 AND task_run_id=?7",
        params![session.session_id, session.document_generation as i64, session.state.persisted(), session.current_step, session.last_snapshot_at_ms, crate::foundation::clock::unix_time_ms_i64(), session.task_run_id],
    ).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Browser automation session persistence was lost.".to_string());
    }
    Ok(())
}

pub(super) fn record_action(
    engine: &PersistenceEngine,
    session: &BrowserSession,
    action_id: &str,
    action_kind: &str,
    reference: Option<&str>,
    state: &str,
    evidence: &serde_json::Value,
    screenshot_path: Option<&str>,
) -> Result<(), String> {
    let now = crate::foundation::clock::unix_time_ms_i64();
    let evidence = serde_json::to_string(evidence).map_err(|error| error.to_string())?;
    engine.open_connection().map_err(|error| error.to_string())?.execute(
        "INSERT INTO browser_automation_actions (action_id, session_id, action_kind, reference_id, destination_origin, state, evidence_json, screenshot_path, created_at_ms, updated_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9) ON CONFLICT(action_id) DO UPDATE SET state=excluded.state,evidence_json=excluded.evidence_json,screenshot_path=excluded.screenshot_path,updated_at_ms=excluded.updated_at_ms",
        params![action_id, session.session_id, action_kind, reference, session.canonical_origin, state, evidence, screenshot_path, now],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn insert_download(
    engine: &PersistenceEngine,
    session: &BrowserSession,
    download: &BrowserDownloadView,
    private_path: &str,
) -> Result<(), String> {
    engine.open_connection().map_err(|error| error.to_string())?.execute(
        "INSERT OR REPLACE INTO browser_download_quarantine (download_id, session_id, source_origin, private_path, file_name, mime_type, byte_count, sha256, state, created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![download.download_id, session.session_id, session.canonical_origin, private_path, download.file_name, download.mime_type, download.byte_count as i64, download.sha256, download.state, crate::foundation::clock::unix_time_ms_i64()],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn list_downloads(
    engine: &PersistenceEngine,
    session_id: &str,
) -> Result<Vec<BrowserDownloadView>, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection.prepare("SELECT download_id,file_name,mime_type,byte_count,sha256,state FROM browser_download_quarantine WHERE session_id=?1 ORDER BY created_at_ms DESC LIMIT 100").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok(BrowserDownloadView {
                download_id: row.get(0)?,
                file_name: row.get(1)?,
                mime_type: row.get(2)?,
                byte_count: row.get::<_, i64>(3)?.max(0) as u64,
                sha256: row.get(4)?,
                state: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

pub(super) fn download_path(
    engine: &PersistenceEngine,
    download_id: &str,
    session_id: &str,
) -> Result<(String, String), String> {
    engine.open_connection().map_err(|error| error.to_string())?.query_row(
        "SELECT private_path,file_name FROM browser_download_quarantine WHERE download_id=?1 AND session_id=?2 AND state='quarantined'",
        params![download_id,session_id], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional().map_err(|error| error.to_string())?.ok_or_else(|| "Quarantined download was not found.".to_string())
}

pub(super) fn mark_download_exported(
    engine: &PersistenceEngine,
    download_id: &str,
) -> Result<(), String> {
    let changed = engine.open_connection().map_err(|error| error.to_string())?.execute(
        "UPDATE browser_download_quarantine SET state='exported',exported_at_ms=?2 WHERE download_id=?1 AND state='quarantined'",
        params![download_id,crate::foundation::clock::unix_time_ms_i64()],
    ).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Download export state changed before completion.".to_string());
    }
    Ok(())
}

#[allow(dead_code)]
pub(super) fn persisted_session(
    engine: &PersistenceEngine,
    session_id: &str,
) -> Result<Option<BrowserSessionView>, String> {
    let _ = (engine, session_id);
    Ok(None)
}
