use super::WorkbookVerification;
use crate::{db::PersistenceEngine, p0_contracts::EvidenceClass, tasks};
use rusqlite::params;

pub(crate) fn mark_review_event_recorded(
    engine: &PersistenceEngine,
    artifact_id: &str,
    revision: u32,
) -> Result<(), String> {
    let changed = engine.open_connection().map_err(|error| error.to_string())?.execute("UPDATE workbook_revisions SET review_event_status_code='recorded',review_event_last_error=NULL WHERE artifact_id=?1 AND revision=?2 AND status_code!='building'", params![artifact_id,revision as i64]).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(
            "Completed workbook revision was not available for evidence attachment.".to_string(),
        );
    }
    Ok(())
}

pub(crate) fn mark_review_event_pending(
    engine: &PersistenceEngine,
    artifact_id: &str,
    revision: u32,
) -> Result<(), String> {
    engine.open_connection().map_err(|error| error.to_string())?.execute("UPDATE workbook_revisions SET review_event_status_code='pending',review_event_last_error='workbook_event_failed' WHERE artifact_id=?1 AND revision=?2 AND review_event_status_code!='recorded'", params![artifact_id,revision as i64]).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn reconcile_review_events(engine: &PersistenceEngine) -> Result<usize, String> {
    let pending = {
        let connection = engine
            .open_connection()
            .map_err(|error| error.to_string())?;
        let mut statement = connection.prepare("SELECT r.artifact_id,v.revision,r.task_run_id,v.status_code,v.xlsx_sha256,v.verification_json,v.manifest_signature_json FROM workbook_records r JOIN workbook_revisions v ON v.artifact_id=r.artifact_id WHERE v.review_event_status_code='pending' AND v.status_code!='building' AND v.xlsx_sha256 IS NOT NULL ORDER BY v.completed_at_ms LIMIT 100").map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u32,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let mut recorded = 0;
    for (artifact_id, revision, task_run_id, status, sha256, verification, signature) in pending {
        if review_event_exists(engine, &task_run_id, &artifact_id, revision)? {
            mark_review_event_recorded(engine, &artifact_id, revision)?;
            recorded += 1;
            continue;
        }
        let verification: WorkbookVerification = serde_json::from_str(&verification)
            .map_err(|_| "Stored workbook verification is invalid.".to_string())?;
        let signature: serde_json::Value = serde_json::from_str(&signature)
            .map_err(|_| "Stored workbook signature is invalid.".to_string())?;
        let evidence = if verification.exportable {
            EvidenceClass::SignedArtifact
        } else {
            EvidenceClass::ObservedResult
        };
        match tasks::record_domain_event(
            engine,
            &task_run_id,
            "workbook.review_ready",
            evidence,
            serde_json::json!({"artifactId":artifact_id,"revision":revision,"statusCode":status,"xlsxSha256":sha256,"exportable":verification.exportable,"manifestSignature":signature}),
        ) {
            Ok(()) => {
                mark_review_event_recorded(engine, &artifact_id, revision)?;
                recorded += 1;
            }
            Err(_) => mark_review_event_pending(engine, &artifact_id, revision)?,
        }
    }
    Ok(recorded)
}

fn review_event_exists(
    engine: &PersistenceEngine,
    task_run_id: &str,
    artifact_id: &str,
    revision: u32,
) -> Result<bool, String> {
    engine.open_connection().map_err(|error| error.to_string())?.query_row("SELECT EXISTS(SELECT 1 FROM task_events WHERE task_run_id=?1 AND json_extract(event_json,'$.eventType')='workbook.review_ready' AND json_extract(event_json,'$.payload.artifactId')=?2 AND json_extract(event_json,'$.payload.revision')=?3)", params![task_run_id,artifact_id,revision as i64], |row| row.get(0)).map_err(|error| error.to_string())
}
