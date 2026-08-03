use super::*;
use crate::{
    db::PersistenceEngine,
    p0_contracts::{ArtifactId, EvidenceClass, ProjectId, TaskId, TaskRunId},
    sovereign_identity::SignatureBlock,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, OptionalExtension};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

pub fn create_presentation_record(
    engine: &PersistenceEngine,
    request: &CreatePresentationRequest,
    evidence: &[BoundPresentationEvidence],
) -> Result<(String, u32), String> {
    engine.require_durable_store("create verified presentation")?;
    let project_id = ProjectId::parse(&request.project_id)?.to_string();
    let task_id = TaskId::parse(&request.task_id)?.to_string();
    let task_run_id = TaskRunId::parse(&request.task_run_id)?.to_string();
    validate_registered_template(
        engine,
        &project_id,
        &task_id,
        &task_run_id,
        &request.presentation,
    )?;
    let presentation_id = ArtifactId::new().to_string();
    let artifact_id = presentation_id.clone();
    let now = crate::foundation::clock::unix_time_ms_i64();
    let ir = serde_json::to_string(&request.presentation).map_err(|error| error.to_string())?;
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO presentation_records (presentation_id,artifact_id,project_id,task_id,task_run_id,title,current_revision,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,?5,?6,1,?7,?7)",
        params![presentation_id,artifact_id,project_id,task_id,task_run_id,request.title,now],
    ).map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO presentation_revisions (presentation_id,revision,presentation_ir_json,scope_code,change_summary,status_code,created_at_ms) VALUES (?1,1,?2,'whole_presentation','Initial presentation','building',?3)",
        params![presentation_id,ir,now],
    ).map_err(|error| error.to_string())?;
    insert_sources(
        &tx,
        &presentation_id,
        1,
        &request.presentation,
        evidence,
        now,
    )?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok((presentation_id, 1))
}

pub fn create_presentation_revision(
    engine: &PersistenceEngine,
    presentation_id: &str,
    base_revision: u32,
    scope: PresentationRevisionScope,
    change_summary: &str,
    presentation: &PresentationIr,
    evidence: &[BoundPresentationEvidence],
) -> Result<u32, String> {
    let presentation_id = ArtifactId::parse(presentation_id)?.to_string();
    let next = base_revision
        .checked_add(1)
        .ok_or_else(|| "Presentation revision limit reached.".to_string())?;
    if presentation.revision != next {
        return Err(
            "Presentation IR revision does not match the next stored revision.".to_string(),
        );
    }
    let ir = serde_json::to_string(presentation).map_err(|error| error.to_string())?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let changed = tx.execute(
        "UPDATE presentation_records SET title=?2,current_revision=?3,updated_at_ms=?4 WHERE presentation_id=?1 AND current_revision=?5",
        params![presentation_id,presentation.title,next as i64,now,base_revision as i64],
    ).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Presentation revision changed; reload before revising.".to_string());
    }
    tx.execute(
        "INSERT INTO presentation_revisions (presentation_id,revision,presentation_ir_json,scope_code,change_summary,status_code,created_at_ms) VALUES (?1,?2,?3,?4,?5,'building',?6)",
        params![presentation_id,next as i64,ir,scope_code(scope),change_summary,now],
    ).map_err(|error| error.to_string())?;
    insert_sources(&tx, &presentation_id, next, presentation, evidence, now)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(next)
}

fn insert_sources(
    connection: &rusqlite::Connection,
    presentation_id: &str,
    revision: u32,
    presentation: &PresentationIr,
    evidence: &[BoundPresentationEvidence],
    now: i64,
) -> Result<(), String> {
    let classes = evidence
        .iter()
        .map(|item| {
            (
                (item.source_ref.as_str(), item.evidence_ref.as_str()),
                item.evidence_class,
            )
        })
        .collect::<HashMap<_, _>>();
    for slide in &presentation.slides {
        for element in &slide.elements {
            for anchor in &element.provenance {
                let class = classes
                    .get(&(anchor.source_ref.as_str(), anchor.evidence_ref.as_str()))
                    .ok_or_else(|| {
                        "Presentation evidence was not bound before persistence.".to_string()
                    })?;
                connection.execute(
                    "INSERT INTO presentation_source_links (presentation_id,revision,slide_id,object_id,source_ref,evidence_ref,evidence_class,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![presentation_id,revision as i64,slide.slide_id,element.object_id,anchor.source_ref,anchor.evidence_ref,evidence_code(*class),now],
                ).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

pub(crate) struct CompletedPresentationRevision<'a> {
    pub presentation_id: &'a str,
    pub revision: u32,
    pub presentation: &'a PresentationIr,
    pub pptx: &'a Path,
    pub previews: &'a [StoredPresentationPreview],
    pub verification: &'a PresentationVerificationRecord,
    pub manifest: &'a serde_json::Value,
    pub signature: &'a SignatureBlock,
    pub pptx_sha256: &'a str,
    pub pptx_bytes: u64,
}

pub(crate) fn complete_presentation_revision(
    engine: &PersistenceEngine,
    completed: CompletedPresentationRevision<'_>,
) -> Result<(), String> {
    let status = if completed.verification.exportable {
        "ready"
    } else {
        "check_required"
    };
    let ir = serde_json::to_string(completed.presentation).map_err(|error| error.to_string())?;
    let previews = serde_json::to_string(completed.previews).map_err(|error| error.to_string())?;
    let verification =
        serde_json::to_string(completed.verification).map_err(|error| error.to_string())?;
    let manifest = serde_json::to_string(completed.manifest).map_err(|error| error.to_string())?;
    let signature =
        serde_json::to_string(completed.signature).map_err(|error| error.to_string())?;
    let changed = engine.open_connection().map_err(|error| error.to_string())?.execute(
        "UPDATE presentation_revisions SET presentation_ir_json=?3,status_code=?4,pptx_private_path=?5,preview_manifest_json=?6,verification_json=?7,manifest_json=?8,manifest_signature_json=?9,pptx_sha256=?10,pptx_bytes=?11,completed_at_ms=?12,last_error=NULL WHERE presentation_id=?1 AND revision=?2 AND status_code='building'",
        params![completed.presentation_id,completed.revision as i64,ir,status,completed.pptx.to_string_lossy(),previews,verification,manifest,signature,completed.pptx_sha256,completed.pptx_bytes as i64,crate::foundation::clock::unix_time_ms_i64()],
    ).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("Presentation completion state changed during verification.".to_string());
    }
    Ok(())
}

pub fn fail_presentation_revision(
    engine: &PersistenceEngine,
    presentation_id: &str,
    revision: u32,
    error: &str,
) -> Result<(), String> {
    engine.open_connection().map_err(|value| value.to_string())?.execute(
        "UPDATE presentation_revisions SET status_code='failed',last_error=?3,completed_at_ms=?4 WHERE presentation_id=?1 AND revision=?2 AND status_code='building'",
        params![presentation_id,revision as i64,error.chars().take(1000).collect::<String>(),crate::foundation::clock::unix_time_ms_i64()],
    ).map_err(|value| value.to_string())?;
    Ok(())
}

pub fn load_presentation_ir(
    engine: &PersistenceEngine,
    presentation_id: &str,
    revision: u32,
) -> Result<PresentationIr, String> {
    let presentation_id = ArtifactId::parse(presentation_id)?.to_string();
    let raw = engine.open_connection().map_err(|error| error.to_string())?.query_row(
        "SELECT presentation_ir_json FROM presentation_revisions WHERE presentation_id=?1 AND revision=?2",
        params![presentation_id,revision as i64],
        |row| row.get::<_,String>(0),
    ).optional().map_err(|error| error.to_string())?.ok_or_else(|| "Presentation revision was not found.".to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

pub fn list_presentation_records(
    engine: &PersistenceEngine,
    request: PresentationListRequest,
) -> Result<Vec<PresentationReviewSummary>, String> {
    let project = request
        .project_id
        .map(ProjectId::parse)
        .transpose()?
        .map(|id| id.to_string());
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection.prepare(
        "SELECT presentation_id FROM presentation_records WHERE (?1 IS NULL OR project_id=?1) ORDER BY updated_at_ms DESC LIMIT 200"
    ).map_err(|error| error.to_string())?;
    let ids = statement
        .query_map(params![project], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    ids.into_iter()
        .map(|id| get_presentation_record(engine, &id, None).map(|detail| detail.summary))
        .collect()
}

pub fn get_presentation_record(
    engine: &PersistenceEngine,
    presentation_id: &str,
    selected: Option<u32>,
) -> Result<PresentationReviewDetail, String> {
    let presentation_id = ArtifactId::parse(presentation_id)?.to_string();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let base = connection.query_row(
        "SELECT artifact_id,project_id,task_id,task_run_id,title,current_revision,updated_at_ms FROM presentation_records WHERE presentation_id=?1",
        params![presentation_id],
        |row| Ok(BaseRecord { artifact_id:row.get(0)?,project_id:row.get(1)?,task_id:row.get(2)?,task_run_id:row.get(3)?,title:row.get(4)?,current_revision:row.get::<_,i64>(5)? as u32,updated_at_ms:row.get(6)? }),
    ).optional().map_err(|error| error.to_string())?.ok_or_else(|| "Presentation was not found.".to_string())?;
    let mut statement = connection.prepare(
        "SELECT revision,presentation_ir_json,scope_code,change_summary,status_code,verification_json,created_at_ms,completed_at_ms,last_error FROM presentation_revisions WHERE presentation_id=?1 ORDER BY revision DESC"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![presentation_id], parse_revision_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let selected_revision = selected.unwrap_or(base.current_revision);
    let selected_row = rows
        .iter()
        .find(|row| row.revision == selected_revision)
        .ok_or_else(|| "Selected presentation revision was not found.".to_string())?;
    let current = rows
        .iter()
        .find(|row| row.revision == base.current_revision)
        .ok_or_else(|| "Current presentation revision is missing.".to_string())?;
    let summary = summary(&presentation_id, &base, current)?;
    let presentation: PresentationIr =
        serde_json::from_str(&selected_row.ir).map_err(|error| error.to_string())?;
    let verification = verification_for(selected_row, presentation.revision)?;
    let filmstrip = filmstrip(&presentation, &verification.issues);
    let provenance = load_provenance(&connection, &presentation_id, selected_revision)?;
    Ok(PresentationReviewDetail {
        summary,
        selected_revision,
        presentation: presentation.clone(),
        revision_history: rows
            .iter()
            .map(revision_summary)
            .collect::<Result<Vec<_>, _>>()?,
        filmstrip,
        issues: verification.issues.clone(),
        notes: presentation
            .slides
            .iter()
            .map(|slide| PresentationNotesItem {
                slide_id: slide.slide_id.clone(),
                speaker_notes: slide.notes.speaker_notes.clone(),
                source_refs: slide.notes.source_refs.clone(),
            })
            .collect(),
        citations: presentation
            .citations
            .iter()
            .map(|citation| PresentationCitationItem {
                citation_id: citation.citation_id.clone(),
                slide_id: citation.slide_id.clone(),
                object_id: citation.object_id.clone(),
                source_ref: citation.source_ref.clone(),
                evidence_ref: citation.evidence_ref.clone(),
                label: citation.label.clone(),
                locator: citation.locator.clone(),
            })
            .collect(),
        provenance,
        template_identity: PresentationTemplateView {
            template_id: presentation.template.template_id.clone(),
            name: presentation.template.name.clone(),
            imported: presentation.template.imported,
            fingerprint_sha256: presentation.template.fingerprint_sha256.clone(),
            master_ids: presentation
                .masters
                .iter()
                .map(|master| master.master_id.clone())
                .collect(),
            layout_ids: presentation
                .layouts
                .iter()
                .map(|layout| layout.layout_id.clone())
                .collect(),
        },
        verification,
    })
}

pub fn load_presentation_preview(
    engine: &PersistenceEngine,
    private_root: &Path,
    request: GetPresentationPreviewRequest,
) -> Result<PresentationPreviewResponse, PresentationCommandError> {
    let detail = get_presentation_record(engine, &request.presentation_id, Some(request.revision))
        .map_err(|error| {
            PresentationCommandError::new("presentation_preview_unavailable", error)
        })?;
    let raw = engine.open_connection().map_err(|error| PresentationCommandError::new("presentation_preview_unavailable",error.to_string()))?.query_row(
        "SELECT preview_manifest_json FROM presentation_revisions WHERE presentation_id=?1 AND revision=?2",
        params![request.presentation_id,request.revision as i64],|row| row.get::<_,Option<String>>(0)
    ).optional().map_err(|error| PresentationCommandError::new("presentation_preview_unavailable",error.to_string()))?.flatten();
    let stored = raw
        .and_then(|value| serde_json::from_str::<Vec<StoredPresentationPreview>>(&value).ok())
        .unwrap_or_default();
    let canonical_root = fs::canonicalize(private_root).map_err(|error| {
        PresentationCommandError::new("presentation_preview_unavailable", error.to_string())
    })?;
    let by_slide = stored
        .into_iter()
        .map(|preview| (preview.slide_id.clone(), preview))
        .collect::<HashMap<_, _>>();
    let mut filmstrip = detail.filmstrip;
    for item in &mut filmstrip {
        if let Some(preview) = by_slide.get(&item.slide_id) {
            let path = Path::new(&preview.path);
            let metadata = fs::symlink_metadata(path).map_err(|_| {
                PresentationCommandError::new(
                    "presentation_preview_unavailable",
                    "Stored slide preview is missing.",
                )
            })?;
            let canonical = fs::canonicalize(path).map_err(|error| {
                PresentationCommandError::new("presentation_preview_unavailable", error.to_string())
            })?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || !canonical.starts_with(&canonical_root)
            {
                return Err(PresentationCommandError::new(
                    "presentation_preview_unavailable",
                    "Stored slide preview failed containment checks.",
                ));
            }
            let bytes = fs::read(canonical).map_err(|error| {
                PresentationCommandError::new("presentation_preview_unavailable", error.to_string())
            })?;
            if super::ooxml::hex_digest(&bytes) != preview.sha256 {
                return Err(PresentationCommandError::new(
                    "presentation_preview_unavailable",
                    "Stored slide preview digest changed.",
                ));
            }
            item.thumbnail = Some(PresentationThumbnail {
                media_type: preview.media_type.clone(),
                bytes_base64: STANDARD.encode(bytes),
                width: preview.width,
                height: preview.height,
            });
        }
    }
    Ok(PresentationPreviewResponse {
        presentation_id: request.presentation_id,
        revision: request.revision,
        filmstrip,
        issues: detail.issues,
        renderer_unavailable: by_slide.is_empty(),
    })
}

pub struct PresentationRevisionFiles {
    pub presentation_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub title: String,
    pub pptx: PathBuf,
    pub sha256: String,
    pub verification: PresentationVerificationRecord,
    pub manifest: serde_json::Value,
    pub signature: SignatureBlock,
}

pub fn presentation_revision_files(
    engine: &PersistenceEngine,
    presentation_id: &str,
    revision: u32,
) -> Result<PresentationRevisionFiles, String> {
    let presentation_id = ArtifactId::parse(presentation_id)?.to_string();
    engine.open_connection().map_err(|error|error.to_string())?.query_row(
        "SELECT r.project_id,r.task_run_id,r.title,v.pptx_private_path,v.pptx_sha256,v.verification_json,v.manifest_json,v.manifest_signature_json FROM presentation_records r JOIN presentation_revisions v ON v.presentation_id=r.presentation_id WHERE r.presentation_id=?1 AND v.revision=?2 AND v.status_code IN ('ready','check_required')",
        params![presentation_id,revision as i64],|row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?))
    ).optional().map_err(|error|error.to_string())?.ok_or_else(||"Presentation revision files are unavailable.".to_string()).and_then(|(project_id,task_run_id,title,path,sha,verification,manifest,signature)|Ok(PresentationRevisionFiles{presentation_id,project_id,task_run_id,title,pptx:PathBuf::from(path),sha256:sha,verification:serde_json::from_str(&verification).map_err(|error|error.to_string())?,manifest:serde_json::from_str(&manifest).map_err(|error|error.to_string())?,signature:serde_json::from_str(&signature).map_err(|error|error.to_string())?}))
}

fn validate_registered_template(
    engine: &PersistenceEngine,
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    presentation: &PresentationIr,
) -> Result<(), String> {
    if !presentation.template.imported {
        return Ok(());
    }
    let template_id = presentation
        .template
        .template_id
        .as_deref()
        .ok_or_else(|| "Imported template ID is missing.".to_string())?;
    let found=engine.open_connection().map_err(|error|error.to_string())?.query_row(
        "SELECT fingerprint_sha256 FROM presentation_template_imports WHERE template_id=?1 AND project_id=?2 AND task_id=?3 AND task_run_id=?4",
        params![template_id,project_id,task_id,task_run_id],|row|row.get::<_,String>(0)
    ).optional().map_err(|error|error.to_string())?;
    if found.as_deref() != Some(&presentation.template.fingerprint_sha256) {
        return Err(
            "Imported presentation template is not registered for this Project.".to_string(),
        );
    }
    Ok(())
}

struct BaseRecord {
    artifact_id: String,
    project_id: String,
    task_id: String,
    task_run_id: String,
    title: String,
    current_revision: u32,
    updated_at_ms: i64,
}
struct RevisionRow {
    revision: u32,
    ir: String,
    scope: String,
    change_summary: String,
    status: String,
    verification: Option<String>,
    created_at_ms: i64,
    completed_at_ms: Option<i64>,
    last_error: Option<String>,
}
fn parse_revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RevisionRow> {
    Ok(RevisionRow {
        revision: row.get::<_, i64>(0)? as u32,
        ir: row.get(1)?,
        scope: row.get(2)?,
        change_summary: row.get(3)?,
        status: row.get(4)?,
        verification: row.get(5)?,
        created_at_ms: row.get(6)?,
        completed_at_ms: row.get(7)?,
        last_error: row.get(8)?,
    })
}
fn summary(
    id: &str,
    base: &BaseRecord,
    row: &RevisionRow,
) -> Result<PresentationReviewSummary, String> {
    let ir: PresentationIr = serde_json::from_str(&row.ir).map_err(|error| error.to_string())?;
    let verification = verification_for(row, ir.revision)?;
    Ok(PresentationReviewSummary {
        presentation_id: id.to_string(),
        project_id: base.project_id.clone(),
        task_id: base.task_id.clone(),
        task_run_id: base.task_run_id.clone(),
        artifact_id: base.artifact_id.clone(),
        title: base.title.clone(),
        current_revision: base.current_revision,
        status: status_code(&row.status)?,
        slide_count: ir.slides.len(),
        issue_count: verification.issues.len(),
        blocker_count: verification
            .issues
            .iter()
            .filter(|issue| issue.severity == PresentationIssueSeverity::Blocker)
            .count(),
        structurally_verified: verification.structurally_verified,
        visually_verified: verification.visually_verified,
        exportable: verification.exportable,
        updated_at_ms: base.updated_at_ms,
    })
}
fn verification_for(
    row: &RevisionRow,
    revision: u32,
) -> Result<PresentationVerificationRecord, String> {
    if let Some(raw) = &row.verification {
        return serde_json::from_str(raw).map_err(|error| error.to_string());
    }
    let failed = row.status == "failed";
    Ok(PresentationVerificationRecord {
        package_sha256: "0".repeat(64),
        structurally_verified: false,
        visually_verified: false,
        exportable: false,
        checked_at_ms: row.completed_at_ms.unwrap_or(row.created_at_ms),
        renderer: None,
        checks: Vec::new(),
        issues: vec![PresentationReviewIssue {
            issue_id: format!("state-{revision}"),
            revision,
            slide_id: None,
            code: if failed {
                "build_failed"
            } else {
                "build_in_progress"
            }
            .to_string(),
            severity: PresentationIssueSeverity::Blocker,
            message: row.last_error.clone().unwrap_or_else(|| {
                if failed {
                    "Presentation build failed.".to_string()
                } else {
                    "Presentation build is still in progress.".to_string()
                }
            }),
            object_id: None,
            evidence_ref: None,
        }],
    })
}
fn revision_summary(row: &RevisionRow) -> Result<PresentationRevisionSummary, String> {
    let verification = verification_for(row, row.revision)?;
    Ok(PresentationRevisionSummary {
        revision: row.revision,
        created_at_ms: row.created_at_ms,
        scope: parse_scope(&row.scope)?,
        change_summary: row.change_summary.clone(),
        structurally_verified: verification.structurally_verified,
        visually_verified: verification.visually_verified,
        exportable: verification.exportable,
    })
}
fn filmstrip(
    ir: &PresentationIr,
    issues: &[PresentationReviewIssue],
) -> Vec<PresentationFilmstripItem> {
    ir.slides
        .iter()
        .enumerate()
        .map(|(index, slide)| {
            let relevant = issues
                .iter()
                .filter(|issue| issue.slide_id.as_deref() == Some(&slide.slide_id));
            let issue_count = relevant.clone().count();
            let blocker_count = relevant
                .filter(|issue| issue.severity == PresentationIssueSeverity::Blocker)
                .count();
            PresentationFilmstripItem {
                slide_id: slide.slide_id.clone(),
                position: index,
                title: slide
                    .title
                    .clone()
                    .unwrap_or_else(|| format!("Slide {}", index + 1)),
                layout_id: slide.layout_id.clone(),
                thumbnail: None,
                issue_count,
                blocker_count,
            }
        })
        .collect()
}
fn load_provenance(
    connection: &rusqlite::Connection,
    id: &str,
    revision: u32,
) -> Result<Vec<PresentationProvenanceItem>, String> {
    let mut statement=connection.prepare("SELECT slide_id,object_id,source_ref,evidence_ref,evidence_class FROM presentation_source_links WHERE presentation_id=?1 AND revision=?2 ORDER BY slide_id,object_id,source_ref").map_err(|error|error.to_string())?;
    let rows = statement
        .query_map(params![id, revision as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .map(|row| {
            let (slide_id, object_id, source_ref, evidence_ref, class) =
                row.map_err(|error| error.to_string())?;
            Ok(PresentationProvenanceItem {
                slide_id,
                object_id,
                source_ref,
                evidence_ref,
                evidence_class: parse_evidence(&class)?,
            })
        })
        .collect();
    rows
}
fn scope_code(value: PresentationRevisionScope) -> &'static str {
    match value {
        PresentationRevisionScope::Slide => "slide",
        PresentationRevisionScope::Element => "element",
        PresentationRevisionScope::NarrativeSection => "narrative_section",
        PresentationRevisionScope::Notes => "notes",
        PresentationRevisionScope::Citations => "citations",
        PresentationRevisionScope::Theme => "theme",
        PresentationRevisionScope::WholePresentation => "whole_presentation",
    }
}
fn parse_scope(value: &str) -> Result<PresentationRevisionScope, String> {
    match value {
        "slide" => Ok(PresentationRevisionScope::Slide),
        "element" => Ok(PresentationRevisionScope::Element),
        "narrative_section" => Ok(PresentationRevisionScope::NarrativeSection),
        "notes" => Ok(PresentationRevisionScope::Notes),
        "citations" => Ok(PresentationRevisionScope::Citations),
        "theme" => Ok(PresentationRevisionScope::Theme),
        "whole_presentation" => Ok(PresentationRevisionScope::WholePresentation),
        _ => Err("Stored presentation revision scope is invalid.".to_string()),
    }
}
fn status_code(value: &str) -> Result<PresentationStatus, String> {
    match value {
        "building" => Ok(PresentationStatus::Building),
        "check_required" => Ok(PresentationStatus::CheckRequired),
        "ready" => Ok(PresentationStatus::Ready),
        "failed" => Ok(PresentationStatus::Failed),
        _ => Err("Stored presentation status is invalid.".to_string()),
    }
}
fn evidence_code(value: EvidenceClass) -> &'static str {
    match value {
        EvidenceClass::ModelAssertion => "model_assertion",
        EvidenceClass::ObservedResult => "observed_result",
        EvidenceClass::ExecutedMutation => "executed_mutation",
        EvidenceClass::VerifiedPostcondition => "verified_postcondition",
        EvidenceClass::SignedArtifact => "signed_artifact",
    }
}
fn parse_evidence(value: &str) -> Result<EvidenceClass, String> {
    match value {
        "model_assertion" => Ok(EvidenceClass::ModelAssertion),
        "observed_result" => Ok(EvidenceClass::ObservedResult),
        "executed_mutation" => Ok(EvidenceClass::ExecutedMutation),
        "verified_postcondition" => Ok(EvidenceClass::VerifiedPostcondition),
        "signed_artifact" => Ok(EvidenceClass::SignedArtifact),
        _ => Err("Stored presentation evidence class is invalid.".to_string()),
    }
}
