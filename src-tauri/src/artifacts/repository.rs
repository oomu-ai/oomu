use super::*;
use crate::{
    db::PersistenceEngine,
    p0_contracts::{ArtifactId, ProjectId, TaskRunId},
};
use rusqlite::{params, OptionalExtension, Row};
use std::path::Path;

pub(super) struct VersionPaths {
    pub artifact_id: String,
    pub title: String,
    pub project_id: String,
    pub task_run_id: String,
    pub docx: PathBuf,
    pub pdf: PathBuf,
    pub manifest: serde_json::Value,
    pub signature: SignatureBlock,
    pub docx_sha256: String,
    pub pdf_sha256: String,
}

pub(super) fn create_record(
    engine: &PersistenceEngine,
    project_id: &str,
    task_run_id: &str,
    document: &ArtifactDocument,
) -> Result<(String, u32), String> {
    engine.require_durable_store("create verified artifact")?;
    let project_id = ProjectId::parse(project_id)?.to_string();
    let task_run_id = TaskRunId::parse(task_run_id)?.to_string();
    let artifact_id = ArtifactId::new().to_string();
    let now = crate::foundation::clock::unix_time_ms_i64();
    let document_json = serde_json::to_string(document).map_err(|error| error.to_string())?;
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    tx.execute("INSERT INTO artifact_records (artifact_id,project_id,task_run_id,title,current_version,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,1,?5,?5)",params![artifact_id,project_id,task_run_id,document.metadata.title,now]).map_err(|error|error.to_string())?;
    tx.execute("INSERT INTO artifact_versions (artifact_id,version,document_json,status,builder_identity,created_at_ms) VALUES (?1,1,?2,'building',?3,?4)",params![artifact_id,document_json,ARTIFACT_BUILDER_IDENTITY,now]).map_err(|error|error.to_string())?;
    insert_sources(&tx, &artifact_id, 1, document, now)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok((artifact_id, 1))
}

pub(super) fn create_revision(
    engine: &PersistenceEngine,
    artifact_id: &str,
    project_id: &str,
    task_run_id: &str,
    instruction: &str,
    document: &ArtifactDocument,
) -> Result<u32, String> {
    let artifact_id = ArtifactId::parse(artifact_id)?.to_string();
    let record = get(engine, &artifact_id)?;
    if record.project_id != project_id || record.task_run_id != task_run_id {
        return Err("Artifact revision scope does not match its Project and Task.".to_string());
    }
    let instruction = instruction.trim();
    if instruction.is_empty() || instruction.chars().count() > 2000 {
        return Err("Artifact revision instruction is invalid.".to_string());
    }
    let version = record
        .current_version
        .checked_add(1)
        .ok_or_else(|| "Artifact version limit reached.".to_string())?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    let document_json = serde_json::to_string(document).map_err(|error| error.to_string())?;
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    tx.execute("INSERT INTO artifact_versions (artifact_id,version,document_json,revision_instruction,status,builder_identity,created_at_ms) VALUES (?1,?2,?3,?4,'building',?5,?6)",params![artifact_id,version as i64,document_json,instruction,ARTIFACT_BUILDER_IDENTITY,now]).map_err(|error|error.to_string())?;
    tx.execute("UPDATE artifact_records SET title=?2,current_version=?3,updated_at_ms=?4 WHERE artifact_id=?1",params![artifact_id,document.metadata.title,version as i64,now]).map_err(|error|error.to_string())?;
    insert_sources(&tx, &artifact_id, version, document, now)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(version)
}

fn insert_sources(
    connection: &rusqlite::Connection,
    artifact_id: &str,
    version: u32,
    document: &ArtifactDocument,
    now: i64,
) -> Result<(), String> {
    for source in document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .flat_map(ArtifactBlock::sources)
    {
        connection.execute("INSERT OR IGNORE INTO artifact_source_links (artifact_id,version,source_ref,evidence_ref,created_at_ms) VALUES (?1,?2,?3,?4,?5)",params![artifact_id,version as i64,source.source_ref,source.evidence_ref,now]).map_err(|error|error.to_string())?;
    }
    for block in document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
    {
        if let ArtifactBlock::Citation {
            source_ref,
            evidence_ref,
            ..
        } = block
        {
            connection.execute("INSERT OR IGNORE INTO artifact_source_links (artifact_id,version,source_ref,evidence_ref,created_at_ms) VALUES (?1,?2,?3,?4,?5)",params![artifact_id,version as i64,source_ref,evidence_ref,now]).map_err(|error|error.to_string())?;
        }
    }
    Ok(())
}

pub(super) struct CompletedVersion<'a> {
    pub artifact_id: &'a str,
    pub version: u32,
    pub docx: &'a Path,
    pub pdf: &'a Path,
    pub previews: &'a [PathBuf],
    pub verification: &'a ArtifactVerification,
    pub provenance: &'a serde_json::Value,
    pub manifest: &'a serde_json::Value,
    pub signature: &'a SignatureBlock,
    pub docx_sha256: &'a str,
    pub pdf_sha256: &'a str,
    pub docx_bytes: u64,
    pub pdf_bytes: u64,
}
pub(super) fn complete(
    engine: &PersistenceEngine,
    complete: CompletedVersion<'_>,
) -> Result<(), String> {
    let preview = serde_json::to_string(
        &complete
            .previews
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| error.to_string())?;
    let verification =
        serde_json::to_string(complete.verification).map_err(|error| error.to_string())?;
    let provenance =
        serde_json::to_string(complete.provenance).map_err(|error| error.to_string())?;
    let manifest = serde_json::to_string(complete.manifest).map_err(|error| error.to_string())?;
    let signature = serde_json::to_string(complete.signature).map_err(|error| error.to_string())?;
    let changed=engine.open_connection().map_err(|error|error.to_string())?.execute("UPDATE artifact_versions SET status='verified',docx_private_path=?3,pdf_private_path=?4,preview_manifest_json=?5,verification_json=?6,provenance_json=?7,manifest_json=?8,manifest_signature_json=?9,docx_sha256=?10,pdf_sha256=?11,docx_bytes=?12,pdf_bytes=?13,renderer_identity=?14,completed_at_ms=?15,last_error=NULL WHERE artifact_id=?1 AND version=?2 AND status IN ('building','verifying')",params![complete.artifact_id,complete.version as i64,complete.docx.to_string_lossy(),complete.pdf.to_string_lossy(),preview,verification,provenance,manifest,signature,complete.docx_sha256,complete.pdf_sha256,complete.docx_bytes as i64,complete.pdf_bytes as i64,ARTIFACT_RENDERER_IDENTITY,crate::foundation::clock::unix_time_ms_i64()]).map_err(|error|error.to_string())?;
    if changed != 1 {
        return Err("Artifact completion state changed during verification.".to_string());
    }
    Ok(())
}
pub(super) fn mark_verifying(
    engine: &PersistenceEngine,
    artifact_id: &str,
    version: u32,
) -> Result<(), String> {
    let changed=engine.open_connection().map_err(|error|error.to_string())?.execute("UPDATE artifact_versions SET status='verifying' WHERE artifact_id=?1 AND version=?2 AND status='building'",params![artifact_id,version as i64]).map_err(|error|error.to_string())?;
    if changed != 1 {
        return Err("Artifact build state changed before verification.".to_string());
    }
    Ok(())
}
pub(super) fn fail(
    engine: &PersistenceEngine,
    artifact_id: &str,
    version: u32,
    error: &str,
) -> Result<(), String> {
    engine.open_connection().map_err(|value|value.to_string())?.execute("UPDATE artifact_versions SET status='failed',last_error=?3,completed_at_ms=?4 WHERE artifact_id=?1 AND version=?2",params![artifact_id,version as i64,error.chars().take(1000).collect::<String>(),crate::foundation::clock::unix_time_ms_i64()]).map_err(|value|value.to_string())?;
    Ok(())
}

pub(super) fn list(
    engine: &PersistenceEngine,
    request: ArtifactListRequest,
) -> Result<Vec<ArtifactRecord>, String> {
    let project = request
        .project_id
        .map(ProjectId::parse)
        .transpose()?
        .map(|id| id.to_string());
    let task = request
        .task_run_id
        .map(TaskRunId::parse)
        .transpose()?
        .map(|id| id.to_string());
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare("SELECT artifact_id FROM artifact_records ORDER BY updated_at_ms DESC LIMIT 200")
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    ids.into_iter()
        .map(|id| get(engine, &id))
        .filter(|record| {
            record.as_ref().is_ok_and(|value| {
                project.as_ref().is_none_or(|id| &value.project_id == id)
                    && task.as_ref().is_none_or(|id| &value.task_run_id == id)
            })
        })
        .collect()
}
pub(super) fn get(engine: &PersistenceEngine, artifact_id: &str) -> Result<ArtifactRecord, String> {
    let artifact_id = ArtifactId::parse(artifact_id)?.to_string();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let base=connection.query_row("SELECT artifact_id,project_id,task_run_id,title,current_version,created_at_ms,updated_at_ms FROM artifact_records WHERE artifact_id=?1",params![artifact_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?))).optional().map_err(|error|error.to_string())?.ok_or_else(||"Artifact was not found.".to_string())?;
    let mut statement=connection.prepare("SELECT version,revision_instruction,status,document_json,preview_manifest_json,verification_json,provenance_json,docx_bytes,pdf_bytes,docx_sha256,pdf_sha256,builder_identity,renderer_identity,created_at_ms,completed_at_ms,last_error,manifest_signature_json FROM artifact_versions WHERE artifact_id=?1 ORDER BY version DESC").map_err(|error|error.to_string())?;
    let versions = statement
        .query_map(params![artifact_id], version_from_row)
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(ArtifactRecord {
        artifact_id: base.0,
        project_id: base.1,
        task_run_id: base.2,
        title: base.3,
        current_version: base.4 as u32,
        created_at_ms: base.5,
        updated_at_ms: base.6,
        versions,
    })
}
fn version_from_row(row: &Row<'_>) -> rusqlite::Result<ArtifactVersion> {
    fn json<T: serde::de::DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
        let raw: String = row.get(index)?;
        serde_json::from_str(&raw).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    }
    let signature_raw: Option<String> = row.get(16)?;
    let signature = signature_raw
        .map(|raw| {
            serde_json::from_str(&raw).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    16,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()?;
    Ok(ArtifactVersion {
        version: row.get::<_, i64>(0)? as u32,
        revision_instruction: row.get(1)?,
        status: row.get(2)?,
        document: json(row, 3)?,
        preview_pages: json(row, 4)?,
        verification: json(row, 5)?,
        provenance: json(row, 6)?,
        docx_bytes: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
        pdf_bytes: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
        docx_sha256: row.get(9)?,
        pdf_sha256: row.get(10)?,
        builder_identity: row.get(11)?,
        renderer_identity: row.get(12)?,
        created_at_ms: row.get(13)?,
        completed_at_ms: row.get(14)?,
        last_error: row.get(15)?,
        manifest_signature: signature,
    })
}

pub(super) fn preview_path(
    engine: &PersistenceEngine,
    artifact_id: &str,
    version: u32,
    page: usize,
) -> Result<PathBuf, String> {
    let record = get(engine, artifact_id)?;
    let selected = record
        .versions
        .into_iter()
        .find(|value| value.version == version && value.status == "verified")
        .ok_or_else(|| "Verified artifact version was not found.".to_string())?;
    selected
        .preview_pages
        .get(page)
        .map(PathBuf::from)
        .ok_or_else(|| "Artifact preview page was not found.".to_string())
}
pub(super) fn version_paths(
    engine: &PersistenceEngine,
    artifact_id: &str,
    version: u32,
) -> Result<VersionPaths, String> {
    let artifact_id = ArtifactId::parse(artifact_id)?.to_string();
    engine.open_connection().map_err(|error|error.to_string())?.query_row("SELECT r.artifact_id,r.title,r.project_id,r.task_run_id,v.docx_private_path,v.pdf_private_path,v.manifest_json,v.manifest_signature_json,v.docx_sha256,v.pdf_sha256 FROM artifact_records r JOIN artifact_versions v ON v.artifact_id=r.artifact_id WHERE r.artifact_id=?1 AND v.version=?2 AND v.status='verified'",params![artifact_id,version as i64],|row|{let manifest:String=row.get(6)?;let signature:String=row.get(7)?;Ok(VersionPaths{artifact_id:row.get(0)?,title:row.get(1)?,project_id:row.get(2)?,task_run_id:row.get(3)?,docx:PathBuf::from(row.get::<_,String>(4)?),pdf:PathBuf::from(row.get::<_,String>(5)?),manifest:serde_json::from_str(&manifest).map_err(|error|rusqlite::Error::FromSqlConversionFailure(6,rusqlite::types::Type::Text,Box::new(error)))?,signature:serde_json::from_str(&signature).map_err(|error|rusqlite::Error::FromSqlConversionFailure(7,rusqlite::types::Type::Text,Box::new(error)))?,docx_sha256:row.get(8)?,pdf_sha256:row.get(9)?})}).optional().map_err(|error|error.to_string())?.ok_or_else(||"Verified artifact version was not found.".to_string())
}
pub(super) fn record_export(
    engine: &PersistenceEngine,
    artifact_id: &str,
    version: u32,
    format: &str,
    destination_hash: &str,
    hashes: &HashMap<String, String>,
) -> Result<(), String> {
    engine.open_connection().map_err(|error|error.to_string())?.execute("INSERT INTO artifact_exports (export_id,artifact_id,version,format,destination_hash,exported_hashes_json,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![format!("export_{}",hex::encode(rand_bytes())),artifact_id,version as i64,format,destination_hash,serde_json::to_string(hashes).map_err(|error|error.to_string())?,crate::foundation::clock::unix_time_ms_i64()]).map_err(|error|error.to_string())?;
    Ok(())
}
fn rand_bytes() -> [u8; 16] {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes
}
