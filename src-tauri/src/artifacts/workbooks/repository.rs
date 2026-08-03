pub(crate) use super::review_events::{
    mark_review_event_pending, mark_review_event_recorded, reconcile_review_events,
};
use super::review_format::{bounded_display, status_code};
use super::*;
use crate::{
    db::PersistenceEngine,
    p0_contracts::{ArtifactId, ProjectId, TaskId, TaskRunId},
    sovereign_identity::SignatureBlock,
};
use rusqlite::{params, OptionalExtension};
use std::path::{Path, PathBuf};

pub(crate) fn create_record(
    engine: &PersistenceEngine,
    request: &CreateWorkbookRequest,
) -> Result<(String, u32), String> {
    engine.require_durable_store("create verified workbook")?;
    let project_id = ProjectId::parse(&request.project_id)?.to_string();
    let task_id = TaskId::parse(&request.task_id)?.to_string();
    let task_run_id = TaskRunId::parse(&request.task_run_id)?.to_string();
    let artifact_id = ArtifactId::new().to_string();
    let now = crate::foundation::clock::unix_time_ms_i64();
    let workbook = serde_json::to_string(&request.workbook).map_err(|error| error.to_string())?;
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    tx.execute("INSERT INTO workbook_records (artifact_id,project_id,task_id,task_run_id,title,current_revision,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,?5,1,?6,?6)", params![artifact_id,project_id,task_id,task_run_id,request.workbook.title,now]).map_err(|error| error.to_string())?;
    tx.execute("INSERT INTO workbook_revisions (artifact_id,revision,workbook_ir_json,status_code,created_at_ms) VALUES (?1,1,?2,'building',?3)", params![artifact_id,workbook,now]).map_err(|error| error.to_string())?;
    insert_sources(&tx, &artifact_id, 1, &request.workbook, now)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok((artifact_id, 1))
}

pub(crate) fn create_revision(
    engine: &PersistenceEngine,
    artifact_id: &str,
    base_revision: u32,
    instruction: &str,
    workbook: &WorkbookIr,
) -> Result<u32, String> {
    let artifact_id = ArtifactId::parse(artifact_id)?.to_string();
    let current = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT current_revision FROM workbook_records WHERE artifact_id=?1",
            params![artifact_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Workbook was not found.".to_string())? as u32;
    if current != base_revision {
        return Err("Workbook revision changed; reload before revising.".to_string());
    }
    let revision = current
        .checked_add(1)
        .ok_or_else(|| "Workbook revision limit reached.".to_string())?;
    if workbook.revision != revision {
        return Err("Workbook IR revision does not match the next stored revision.".to_string());
    }
    let workbook_json = serde_json::to_string(workbook).map_err(|error| error.to_string())?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let changed = tx.execute("UPDATE workbook_records SET title=?2,current_revision=?3,updated_at_ms=?4 WHERE artifact_id=?1 AND current_revision=?5", params![artifact_id,workbook.title,revision as i64,now,base_revision as i64]).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Workbook revision changed; reload before revising.".to_string());
    }
    tx.execute("INSERT INTO workbook_revisions (artifact_id,revision,workbook_ir_json,revision_instruction,status_code,created_at_ms) VALUES (?1,?2,?3,?4,'building',?5)", params![artifact_id,revision as i64,workbook_json,instruction,now]).map_err(|error| error.to_string())?;
    insert_sources(&tx, &artifact_id, revision, workbook, now)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(revision)
}

fn insert_sources(
    connection: &rusqlite::Connection,
    artifact_id: &str,
    revision: u32,
    workbook: &WorkbookIr,
    now: i64,
) -> Result<(), String> {
    for sheet in &workbook.worksheets {
        for cell in &sheet.cells {
            for source in &cell.provenance {
                connection.execute("INSERT OR IGNORE INTO workbook_source_links (artifact_id,revision,sheet_id,cell_address,source_ref,evidence_ref,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![artifact_id,revision as i64,sheet.sheet_id,cell.address,source.source_ref,source.evidence_ref,now]).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

pub(crate) struct CompletedRevision<'a> {
    pub artifact_id: &'a str,
    pub revision: u32,
    pub workbook: &'a WorkbookIr,
    pub xlsx: &'a Path,
    pub previews: &'a [StoredWorkbookPreview],
    pub verification: &'a WorkbookVerification,
    pub manifest: &'a serde_json::Value,
    pub signature: &'a SignatureBlock,
    pub xlsx_sha256: &'a str,
    pub xlsx_bytes: u64,
}

pub(crate) fn complete(
    engine: &PersistenceEngine,
    completed: CompletedRevision<'_>,
) -> Result<(), String> {
    let status = match completed.verification.status_code {
        WorkbookStatusCode::Ready => "ready",
        WorkbookStatusCode::NeedsRecalculation => "needs_recalculation",
        WorkbookStatusCode::CheckRequired | WorkbookStatusCode::Building => "check_required",
        WorkbookStatusCode::Failed => "failed",
    };
    let workbook = serde_json::to_string(completed.workbook).map_err(|error| error.to_string())?;
    let previews = serde_json::to_string(completed.previews).map_err(|error| error.to_string())?;
    let verification =
        serde_json::to_string(completed.verification).map_err(|error| error.to_string())?;
    let manifest = serde_json::to_string(completed.manifest).map_err(|error| error.to_string())?;
    let signature =
        serde_json::to_string(completed.signature).map_err(|error| error.to_string())?;
    let changed = engine.open_connection().map_err(|error| error.to_string())?.execute("UPDATE workbook_revisions SET workbook_ir_json=?3,status_code=?4,xlsx_private_path=?5,preview_manifest_json=?6,verification_json=?7,manifest_json=?8,manifest_signature_json=?9,xlsx_sha256=?10,xlsx_bytes=?11,completed_at_ms=?12,last_error=NULL,review_event_status_code='pending',review_event_last_error=NULL WHERE artifact_id=?1 AND revision=?2 AND status_code='building'", params![completed.artifact_id,completed.revision as i64,workbook,status,completed.xlsx.to_string_lossy(),previews,verification,manifest,signature,completed.xlsx_sha256,completed.xlsx_bytes as i64,crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Workbook completion state changed during verification.".to_string());
    }
    Ok(())
}

pub(crate) fn fail(
    engine: &PersistenceEngine,
    artifact_id: &str,
    revision: u32,
    error: &str,
) -> Result<(), String> {
    engine.open_connection().map_err(|value| value.to_string())?.execute("UPDATE workbook_revisions SET status_code='failed',last_error=?3,completed_at_ms=?4 WHERE artifact_id=?1 AND revision=?2 AND status_code='building'", params![artifact_id,revision as i64,error.chars().take(1000).collect::<String>(),crate::foundation::clock::unix_time_ms_i64()]).map_err(|value| value.to_string())?;
    Ok(())
}

pub(crate) fn load_ir(
    engine: &PersistenceEngine,
    artifact_id: &str,
    revision: u32,
) -> Result<WorkbookIr, String> {
    let artifact_id = ArtifactId::parse(artifact_id)?.to_string();
    let raw = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT workbook_ir_json FROM workbook_revisions WHERE artifact_id=?1 AND revision=?2",
            params![artifact_id, revision as i64],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Workbook revision was not found.".to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

pub(crate) fn list(
    engine: &PersistenceEngine,
    request: WorkbookListRequest,
) -> Result<Vec<WorkbookReviewSummary>, String> {
    let project = request
        .project_id
        .map(ProjectId::parse)
        .transpose()?
        .map(|value| value.to_string());
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection.prepare("SELECT artifact_id FROM workbook_records WHERE (?1 IS NULL OR project_id=?1) ORDER BY updated_at_ms DESC LIMIT 200").map_err(|error| error.to_string())?;
    let ids = statement
        .query_map(params![project], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    ids.into_iter()
        .map(|id| get(engine, &id).map(|record| summary(&record)))
        .collect()
}

fn summary(record: &WorkbookReviewRecord) -> WorkbookReviewSummary {
    let current = record
        .revisions
        .iter()
        .find(|revision| revision.revision == record.current_revision);
    WorkbookReviewSummary {
        artifact_id: record.artifact_id.clone(),
        project_id: record.project_id.clone(),
        task_id: record.task_id.clone(),
        task_run_id: record.task_run_id.clone(),
        title: record.title.clone(),
        current_revision: record.current_revision,
        status_code: current
            .map(|value| value.status_code)
            .unwrap_or(WorkbookStatusCode::CheckRequired),
        preview_available: record.preview_available,
        safe_prior_revision: record.safe_prior_revision,
        updated_at_ms: record.updated_at_ms,
    }
}

pub(crate) fn get(
    engine: &PersistenceEngine,
    artifact_id: &str,
) -> Result<WorkbookReviewRecord, String> {
    let artifact_id = ArtifactId::parse(artifact_id)?.to_string();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let base = connection.query_row("SELECT artifact_id,project_id,task_id,task_run_id,title,current_revision,created_at_ms,updated_at_ms FROM workbook_records WHERE artifact_id=?1", params![artifact_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?,row.get::<_,i64>(7)?))).optional().map_err(|error| error.to_string())?.ok_or_else(|| "Workbook was not found.".to_string())?;
    let mut statement = connection.prepare("SELECT revision,status_code,workbook_ir_json,preview_manifest_json,verification_json,created_at_ms,completed_at_ms,last_error,xlsx_private_path,xlsx_sha256 FROM workbook_revisions WHERE artifact_id=?1 ORDER BY revision DESC").map_err(|error| error.to_string())?;
    let revisions = statement
        .query_map(params![artifact_id], |row| {
            let revision = row.get::<_, i64>(0)? as u32;
            let status: String = row.get(1)?;
            let workbook: String = row.get(2)?;
            let previews: String = row.get(3)?;
            let verification: Option<String> = row.get(4)?;
            let last_error: Option<String> = row.get(7)?;
            revision_view(
                revision,
                &status,
                &workbook,
                &previews,
                verification.as_deref(),
                row.get(5)?,
                row.get(6)?,
                last_error.as_deref(),
                row.get::<_, Option<String>>(8)?.is_some(),
                row.get::<_, Option<String>>(9)?.is_some(),
            )
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
                )
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let current = revisions
        .iter()
        .find(|revision| revision.revision == base.5 as u32);
    let usable = current
        .filter(|revision| revision.sheets.iter().any(|sheet| sheet.preview_available))
        .or_else(|| {
            revisions
                .iter()
                .find(|revision| revision.sheets.iter().any(|sheet| sheet.preview_available))
        });
    let selected_sheet_id = usable
        .and_then(|revision| {
            revision
                .sheets
                .iter()
                .find(|sheet| sheet.preview_available)
                .or_else(|| revision.sheets.first())
        })
        .map(|sheet| sheet.sheet_id.clone());
    let preview_available = revisions
        .iter()
        .any(|revision| revision.sheets.iter().any(|sheet| sheet.preview_available));
    let safe_prior_revision = revisions
        .iter()
        .filter(|revision| revision.revision < base.5 as u32 && revision.recoverable)
        .map(|revision| revision.revision)
        .max();
    Ok(WorkbookReviewRecord {
        artifact_id: base.0,
        project_id: base.1,
        task_id: base.2,
        task_run_id: base.3,
        title: base.4,
        current_revision: base.5 as u32,
        selected_sheet_id,
        preview_available,
        safe_prior_revision,
        created_at_ms: base.6,
        updated_at_ms: base.7,
        revisions,
    })
}

fn revision_view(
    revision: u32,
    status: &str,
    workbook_raw: &str,
    previews_raw: &str,
    verification_raw: Option<&str>,
    created_at_ms: i64,
    completed_at_ms: Option<i64>,
    last_error: Option<&str>,
    has_output: bool,
    has_digest: bool,
) -> Result<WorkbookRevisionView, String> {
    let workbook: WorkbookIr =
        serde_json::from_str(workbook_raw).map_err(|error| error.to_string())?;
    let previews: Vec<StoredWorkbookPreview> =
        serde_json::from_str(previews_raw).map_err(|error| error.to_string())?;
    let verification = verification_raw
        .map(serde_json::from_str::<WorkbookVerification>)
        .transpose()
        .map_err(|error| error.to_string())?;
    let preview_ids = previews
        .iter()
        .map(|preview| preview.sheet_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let sheets = workbook
        .worksheets
        .iter()
        .map(|sheet| WorkbookSheetView {
            sheet_id: sheet.sheet_id.clone(),
            name: sheet.name.clone(),
            preview_available: preview_ids.contains(sheet.sheet_id.as_str()),
        })
        .collect();
    let formula_cells = workbook
        .worksheets
        .iter()
        .flat_map(|sheet| {
            sheet.cells.iter().filter_map(|cell| match &cell.value {
                CellValue::Formula {
                    expression,
                    cached_value,
                } => {
                    let (display, status_code) = match cached_value {
                        Some(FormulaResult::Number { value }) => {
                            (value.to_string(), WorkbookFormulaStatusCode::UpToDate)
                        }
                        Some(FormulaResult::Text { value }) => {
                            (value.clone(), WorkbookFormulaStatusCode::UpToDate)
                        }
                        Some(FormulaResult::Boolean { value }) => (
                            if *value { "TRUE" } else { "FALSE" }.to_string(),
                            WorkbookFormulaStatusCode::UpToDate,
                        ),
                        Some(FormulaResult::Error { code }) => {
                            (code.clone(), WorkbookFormulaStatusCode::Error)
                        }
                        None => (String::new(), WorkbookFormulaStatusCode::NeedsRecalculation),
                    };
                    Some(WorkbookFormulaCellView {
                        sheet_id: sheet.sheet_id.clone(),
                        address: cell.address.clone(),
                        expression: bounded_display(expression, 512),
                        display_value: bounded_display(&display, 240),
                        status_code,
                    })
                }
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    let lineage = workbook
        .worksheets
        .iter()
        .flat_map(|sheet| {
            sheet.cells.iter().flat_map(|cell| {
                cell.provenance.iter().map(|source| WorkbookLineageView {
                    sheet_id: sheet.sheet_id.clone(),
                    address: cell.address.clone(),
                    source_ref: source.source_ref.clone(),
                    evidence_ref: source.evidence_ref.clone(),
                })
            })
        })
        .collect();
    let numbers_status_code = if formula_cells.is_empty() {
        WorkbookNumbersStatusCode::NotApplicable
    } else if verification
        .as_ref()
        .is_some_and(|value| value.formulas_verified)
    {
        WorkbookNumbersStatusCode::UpToDate
    } else {
        WorkbookNumbersStatusCode::NeedsRecalculation
    };
    let recoverable = verification.is_some() && has_output && has_digest && !previews.is_empty();
    Ok(WorkbookRevisionView {
        revision,
        status_code: status_code(status),
        created_at_ms,
        completed_at_ms,
        sheets,
        formula_cells,
        lineage,
        warnings: verification
            .as_ref()
            .map(|value| value.warnings.clone())
            .unwrap_or_default(),
        numbers_status_code,
        exportable: verification.as_ref().is_some_and(|value| value.exportable),
        evidence_summary: verification
            .as_ref()
            .map(|value| value.evidence.clone())
            .unwrap_or_default(),
        technical_evidence_available: verification.is_some(),
        recoverable,
        last_error_code: last_error.map(|_| "workbook_revision_failed".to_string()),
    })
}

pub(crate) struct RevisionFiles {
    pub artifact_id: String,
    pub revision: u32,
    pub project_id: String,
    pub task_run_id: String,
    pub title: String,
    pub xlsx: PathBuf,
    pub sha256: String,
    pub manifest: serde_json::Value,
    pub signature: SignatureBlock,
    pub verification: WorkbookVerification,
}

pub(crate) fn revision_files(
    engine: &PersistenceEngine,
    artifact_id: &str,
    revision: u32,
) -> Result<RevisionFiles, String> {
    let artifact_id = ArtifactId::parse(artifact_id)?.to_string();
    engine.open_connection().map_err(|error| error.to_string())?.query_row("SELECT r.artifact_id,v.revision,r.project_id,r.task_run_id,r.title,v.xlsx_private_path,v.xlsx_sha256,v.manifest_json,v.manifest_signature_json,v.verification_json FROM workbook_records r JOIN workbook_revisions v ON v.artifact_id=r.artifact_id WHERE r.artifact_id=?1 AND v.revision=?2 AND v.status_code IN ('ready','needs_recalculation','check_required') AND v.review_event_status_code='recorded'", params![artifact_id,revision as i64], |row| {
        let json = |index| -> rusqlite::Result<String> { row.get(index) };
        Ok(RevisionFiles { artifact_id:row.get(0)?,revision:row.get::<_,i64>(1)? as u32,project_id:row.get(2)?,task_run_id:row.get(3)?,title:row.get(4)?,xlsx:PathBuf::from(row.get::<_,String>(5)?),sha256:row.get(6)?,manifest:serde_json::from_str(&json(7)?).map_err(json_error(7))?,signature:serde_json::from_str(&json(8)?).map_err(json_error(8))?,verification:serde_json::from_str(&json(9)?).map_err(json_error(9))? })
    }).optional().map_err(|error| error.to_string())?.ok_or_else(|| "Completed workbook revision was not found.".to_string())
}

fn json_error(index: usize) -> impl FnOnce(serde_json::Error) -> rusqlite::Error {
    move |error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    }
}

pub(crate) fn preview(
    engine: &PersistenceEngine,
    request: &WorkbookPreviewRequest,
) -> Result<StoredWorkbookPreview, String> {
    let artifact_id = ArtifactId::parse(&request.artifact_id)?.to_string();
    let raw = engine.open_connection().map_err(|error| error.to_string())?.query_row("SELECT preview_manifest_json FROM workbook_revisions WHERE artifact_id=?1 AND revision=?2", params![artifact_id,request.revision as i64], |row| row.get::<_,String>(0)).optional().map_err(|error| error.to_string())?.ok_or_else(|| "Workbook preview was not found.".to_string())?;
    serde_json::from_str::<Vec<StoredWorkbookPreview>>(&raw)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|preview| preview.sheet_id == request.sheet_id)
        .ok_or_else(|| "Workbook sheet preview was not found.".to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExportReceipt {
    pub export_id: String,
}

pub(crate) fn begin_export(
    engine: &PersistenceEngine,
    files: &RevisionFiles,
    destination_hash: &str,
) -> Result<ExportReceipt, String> {
    let export_id = format!("workbook_export_{}", hex::encode(random_bytes()));
    engine.open_connection().map_err(|error| error.to_string())?.execute("INSERT INTO workbook_exports (export_id,artifact_id,revision,destination_hash,xlsx_sha256,created_at_ms,status_code) VALUES (?1,?2,?3,?4,?5,?6,'pending')", params![export_id,files.artifact_id,files.revision as i64,destination_hash,files.sha256,crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
    Ok(ExportReceipt { export_id })
}

pub(crate) fn complete_export(
    engine: &PersistenceEngine,
    receipt: &ExportReceipt,
) -> Result<(), String> {
    let changed = engine.open_connection().map_err(|error| error.to_string())?.execute("UPDATE workbook_exports SET status_code='committed',completed_at_ms=?2,last_error=NULL WHERE export_id=?1 AND status_code='pending'", params![receipt.export_id,crate::foundation::clock::unix_time_ms_i64()]).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Workbook export receipt is no longer pending.".to_string());
    }
    Ok(())
}

pub(crate) fn fail_export(
    engine: &PersistenceEngine,
    receipt: &ExportReceipt,
    error_code: &str,
) -> Result<(), String> {
    engine.open_connection().map_err(|error| error.to_string())?.execute("UPDATE workbook_exports SET status_code='failed',completed_at_ms=?2,last_error=?3 WHERE export_id=?1 AND status_code='pending'", params![receipt.export_id,crate::foundation::clock::unix_time_ms_i64(),error_code.chars().take(120).collect::<String>()]).map_err(|error| error.to_string())?;
    Ok(())
}

fn random_bytes() -> [u8; 18] {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0_u8; 18];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p0_contracts::{
        EvidenceClass, P0EventEnvelope, ProjectId, TaskId, TaskRunId, P0_CONTRACT_VERSION,
    };
    use serde_json::json;

    fn temporary_engine(label: &str) -> (std::path::PathBuf, PersistenceEngine) {
        let root = std::env::temp_dir().join(format!(
            "oomu-workbook-{label}-{}",
            hex::encode(random_bytes())
        ));
        let engine = PersistenceEngine::initialize_volatile_at(root.join("state.sqlite")).unwrap();
        (root, engine)
    }

    #[test]
    fn failure_transition_only_applies_while_revision_is_building() {
        let (root, engine) = temporary_engine("state-transition");
        let artifact_id = ArtifactId::new().to_string();
        let connection = engine.open_connection().unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        connection
            .execute(
                "INSERT INTO workbook_revisions (artifact_id,revision,workbook_ir_json,status_code,created_at_ms) VALUES (?1,1,'{}','building',1)",
                params![artifact_id],
            )
            .unwrap();

        fail(&engine, &artifact_id, 1, "workbook_event_failed").unwrap();
        let failed = connection
            .query_row(
                "SELECT status_code,last_error FROM workbook_revisions WHERE artifact_id=?1 AND revision=1",
                params![artifact_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        assert_eq!(
            failed,
            (
                "failed".to_string(),
                Some("workbook_event_failed".to_string())
            )
        );

        connection
            .execute(
                "UPDATE workbook_revisions SET status_code='ready',last_error=NULL WHERE artifact_id=?1 AND revision=1",
                params![artifact_id],
            )
            .unwrap();
        fail(&engine, &artifact_id, 1, "late_event_failure").unwrap();
        let completed = connection
            .query_row(
                "SELECT status_code,last_error FROM workbook_revisions WHERE artifact_id=?1 AND revision=1",
                params![artifact_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        assert_eq!(completed, ("ready".to_string(), None));
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_receipts_have_one_way_pending_transitions() {
        let (root, engine) = temporary_engine("export-transition");
        let connection = engine.open_connection().unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        for id in ["workbook_export_committed", "workbook_export_failed"] {
            connection.execute("INSERT INTO workbook_exports (export_id,artifact_id,revision,destination_hash,xlsx_sha256,created_at_ms,status_code) VALUES (?1,'artifact_00000000-0000-4000-8000-000000000000',1,?2,?2,1,'pending')", params![id,"a".repeat(64)]).unwrap();
        }
        let committed = ExportReceipt {
            export_id: "workbook_export_committed".to_string(),
        };
        complete_export(&engine, &committed).unwrap();
        fail_export(&engine, &committed, "late_failure").unwrap();
        let committed_state: (String, Option<String>) = connection
            .query_row(
                "SELECT status_code,last_error FROM workbook_exports WHERE export_id=?1",
                params![committed.export_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(committed_state, ("committed".to_string(), None));

        let failed = ExportReceipt {
            export_id: "workbook_export_failed".to_string(),
        };
        fail_export(&engine, &failed, "workbook_export_failed").unwrap();
        assert!(complete_export(&engine, &failed).is_err());
        let failed_state: (String, Option<String>) = connection
            .query_row(
                "SELECT status_code,last_error FROM workbook_exports WHERE export_id=?1",
                params![failed.export_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            failed_state,
            (
                "failed".to_string(),
                Some("workbook_export_failed".to_string())
            )
        );
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn review_lineage_only_exposes_bound_task_evidence() {
        let project = ProjectId::new();
        let task = TaskId::new();
        let run = TaskRunId::new();
        let mut workbook = deterministic_fixture().unwrap();
        workbook.worksheets[0].cells[0].provenance = vec![
            ProvenanceReference {
                source_ref: "connector.read_verified".to_string(),
                evidence_ref: format!("task-event:{}:2", run.as_str()),
                note: None,
            },
            ProvenanceReference {
                source_ref: "forged.source".to_string(),
                evidence_ref: "forged-evidence".to_string(),
                note: None,
            },
        ];
        let event = P0EventEnvelope {
            schema_version: P0_CONTRACT_VERSION,
            event_type: "connector.read_verified".to_string(),
            project_id: project.clone(),
            task_id: task.clone(),
            task_run_id: Some(run.clone()),
            correlation_id: "correlation".to_string(),
            sequence: 2,
            timestamp: "2026-07-11T00:00:00.000Z".to_string(),
            evidence_class: EvidenceClass::VerifiedPostcondition,
            payload: json!({}),
        };
        super::super::provenance::bind_from_events(
            project.as_str(),
            task.as_str(),
            run.as_str(),
            &mut workbook,
            vec![(2, event)],
        )
        .unwrap();
        let view = revision_view(
            1,
            "building",
            &serde_json::to_string(&workbook).unwrap(),
            "[]",
            None,
            1,
            None,
            None,
            false,
            false,
        )
        .unwrap();
        assert_eq!(view.lineage.len(), 1);
        assert_eq!(view.lineage[0].source_ref, "connector.read_verified");
        assert_ne!(view.lineage[0].evidence_ref, "forged-evidence");
    }

    #[test]
    fn review_dto_preserves_building_and_exposes_bounded_formula_state() {
        let output = build_workbook(&deterministic_fixture().unwrap()).unwrap();
        let workbook = serde_json::to_string(&output.workbook).unwrap();
        let building = revision_view(
            1, "building", &workbook, "[]", None, 1, None, None, false, false,
        )
        .unwrap();
        assert_eq!(building.status_code, WorkbookStatusCode::Building);
        assert!(!building.recoverable);
        let preview = StoredWorkbookPreview {
            sheet_id: output.previews[0].evidence.sheet_id.clone(),
            path: "/private/preview.png".into(),
            mime_type: "image/png".into(),
            width: 1200,
            height: 800,
            sha256: output.previews[0].evidence.sha256.clone(),
        };
        let ready = revision_view(
            1,
            "ready",
            &workbook,
            &serde_json::to_string(&vec![preview]).unwrap(),
            Some(&serde_json::to_string(&output.verification).unwrap()),
            1,
            Some(2),
            None,
            true,
            true,
        )
        .unwrap();
        assert!(ready.recoverable);
        let formula = ready
            .formula_cells
            .iter()
            .find(|cell| cell.address == "B5")
            .unwrap();
        assert_eq!(formula.display_value, "3900");
        assert_eq!(formula.status_code, WorkbookFormulaStatusCode::UpToDate);
        assert!(formula.expression.len() <= 512);
        assert_eq!(ready.exportable, output.verification.exportable);
    }

    #[test]
    fn review_event_marker_is_durable_and_cannot_regress() {
        let (root, engine) = temporary_engine("review-event-transition");
        let artifact_id = ArtifactId::new().to_string();
        let connection = engine.open_connection().unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        connection.execute(
            "INSERT INTO workbook_revisions (artifact_id,revision,workbook_ir_json,status_code,created_at_ms,review_event_status_code) VALUES (?1,1,'{}','check_required',1,'pending')",
            params![artifact_id],
        ).unwrap();
        mark_review_event_recorded(&engine, &artifact_id, 1).unwrap();
        mark_review_event_pending(&engine, &artifact_id, 1).unwrap();
        let state: (String, Option<String>) = connection.query_row(
            "SELECT review_event_status_code,review_event_last_error FROM workbook_revisions WHERE artifact_id=?1 AND revision=1",
            params![artifact_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(state, ("recorded".to_string(), None));
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}
