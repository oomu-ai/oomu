use super::*;
use crate::{
    db::PersistenceEngine,
    p0_contracts::{ProjectId, TaskId, TaskRunId},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{GenericImageView, ImageFormat};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension, Row};
use std::io::Cursor;

const MAX_MEDIA_BYTES: usize = 25 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 50_000_000;

fn uuid_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!("{prefix}_{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}", bytes[0],bytes[1],bytes[2],bytes[3],bytes[4],bytes[5],bytes[6],bytes[7],bytes[8],bytes[9],bytes[10],bytes[11],bytes[12],bytes[13],bytes[14],bytes[15])
}

fn validate_request(request: &IngestMediaRequest) -> Result<(String, Vec<u8>), String> {
    ProjectId::parse(&request.project_id)?;
    request.task_id.as_deref().map(TaskId::parse).transpose()?;
    request
        .task_run_id
        .as_deref()
        .map(TaskRunId::parse)
        .transpose()?;
    if request.source_reference.trim().is_empty() || request.source_reference.len() > 1_024 {
        return Err("A bounded media source label is required.".into());
    }
    if !matches!(
        request.source_kind.as_str(),
        "microphone"
            | "voice_message"
            | "screenshot"
            | "clipboard"
            | "camera"
            | "project_file"
            | "generated"
    ) {
        return Err("This media source is not supported.".into());
    }
    if !matches!(
        request.retention_mode.as_str(),
        "task" | "project" | "until"
    ) || (request.retention_mode == "until" && request.expires_at_ms.is_none())
    {
        return Err("The media retention choice is incomplete.".into());
    }
    if request.routing_mode == "local_only" && !request.provider_ids.is_empty() {
        return Err("Local-only media cannot name a cloud provider.".into());
    }
    if !matches!(
        request.routing_mode.as_str(),
        "local_only" | "approved_providers"
    ) {
        return Err("The media routing choice is invalid.".into());
    }
    let bytes = STANDARD
        .decode(request.data_base64.trim())
        .map_err(|_| "The media payload is not valid.".to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_MEDIA_BYTES {
        return Err("The media file is empty or too large.".into());
    }
    let kind = match request.mime_type.as_str() {
        "image/png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => "image",
        "audio/wav" | "audio/x-wav"
            if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") =>
        {
            "audio"
        }
        "audio/webm" | "video/webm" if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) => {
            if request.mime_type.starts_with("audio/") {
                "audio"
            } else {
                "video"
            }
        }
        "audio/mp4" | "video/mp4" if bytes.get(4..8) == Some(b"ftyp") => {
            if request.mime_type.starts_with("audio/") {
                "audio"
            } else {
                "video"
            }
        }
        _ => return Err("The file contents do not match a supported media format.".into()),
    };
    Ok((kind.into(), bytes))
}

fn image_dimensions(bytes: &[u8], kind: &str) -> Result<(Option<u32>, Option<u32>), String> {
    if kind != "image" {
        return Ok((None, None));
    }
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .map_err(|_| "This image could not be decoded safely.".to_string())?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err("This image is too large to process safely.".into());
    }
    Ok((Some(width), Some(height)))
}

fn ensure_project(engine: &PersistenceEngine, project_id: &str) -> Result<(), String> {
    ProjectId::parse(project_id)?;
    let exists = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT 1 FROM projects WHERE project_id=?1 AND archived_at_ms IS NULL",
            params![project_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();
    if !exists {
        return Err("Project was not found.".to_string());
    }
    Ok(())
}

pub fn ingest(
    engine: &PersistenceEngine,
    request: IngestMediaRequest,
) -> Result<MediaAssetRecord, String> {
    engine.require_durable_store("save media")?;
    ensure_project(engine, &request.project_id)?;
    let project_policy: String = engine
        .open_connection()
        .map_err(|e| e.to_string())?
        .query_row(
            "SELECT data_policy FROM project_policy WHERE project_id=?1",
            params![request.project_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if request.routing_mode == "approved_providers" && project_policy != "allow_configured_cloud" {
        return Err("This Project does not allow media to use a cloud service.".into());
    }
    if let Some(task_run_id) = request.task_run_id.as_deref() {
        crate::tasks::require_bound_task(engine, task_run_id, &request.project_id)?;
    }
    let (kind, bytes) = validate_request(&request)?;
    let (width, height) = image_dimensions(&bytes, &kind)?;
    let id = uuid_id("media");
    let digest = crate::foundation::digest::sha256_hex(&bytes);
    let now = crate::foundation::clock::unix_time_ms_i64();
    let redaction_state = if request.redaction_categories.is_empty() {
        "not_required"
    } else {
        "required"
    };
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    connection.execute("INSERT INTO media_assets (media_asset_id,project_id,task_id,task_run_id,media_kind,mime_type,sha256,byte_length,source_kind,source_reference,width,height,duration_ms,retention_mode,expires_at_ms,redaction_state,redaction_categories_json,routing_mode,provider_ids_json,original_blob,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,?13,?14,?15,?16,?17,?18,?19,?20)", params![id,request.project_id,request.task_id,request.task_run_id,kind,request.mime_type,digest,bytes.len() as i64,request.source_kind,request.source_reference,width,height,request.retention_mode,request.expires_at_ms,redaction_state,serde_json::to_string(&request.redaction_categories).map_err(|e|e.to_string())?,request.routing_mode,serde_json::to_string(&request.provider_ids).map_err(|e|e.to_string())?,bytes,now]).map_err(|error| error.to_string())?;
    connection.execute("INSERT INTO media_evidence (evidence_id,media_asset_id,project_id,task_run_id,evidence_class,event_kind,detail_json,created_at_ms) VALUES (?1,?2,?3,?4,'observed_result','media_ingested',?5,?6)", params![uuid_id("evidence"),id,request.project_id,request.task_run_id,serde_json::json!({"sha256":digest,"mimeType":request.mime_type,"byteLength":bytes.len()}).to_string(),now]).map_err(|e|e.to_string())?;
    get(engine, &request.project_id, &id)
}

fn latest_transcript(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<Option<TranscriptRecord>, String> {
    connection.query_row("SELECT revision,transcript,language,confidence,timestamps_json,route_kind,route_label,edited_by_user,created_at_ms FROM media_transcripts WHERE media_asset_id=?1 ORDER BY revision DESC LIMIT 1", params![id], |row| {
        let raw:String=row.get(4)?; Ok(TranscriptRecord{revision:row.get::<_,i64>(0)? as u64,transcript:row.get(1)?,language:row.get(2)?,confidence:row.get(3)?,timestamps:serde_json::from_str(&raw).unwrap_or_default(),route_kind:row.get(5)?,route_label:row.get(6)?,edited_by_user:row.get::<_,i64>(7)?!=0,created_at_ms:row.get(8)?})
    }).optional().map_err(|e|e.to_string())
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<MediaAssetRecord> {
    let categories: String = row.get(15)?;
    let providers: String = row.get(17)?;
    Ok(MediaAssetRecord {
        media_asset_id: row.get(0)?,
        project_id: row.get(1)?,
        task_id: row.get(2)?,
        task_run_id: row.get(3)?,
        media_kind: row.get(4)?,
        mime_type: row.get(5)?,
        sha256: row.get(6)?,
        byte_length: row.get::<_, i64>(7)? as u64,
        source_kind: row.get(8)?,
        source_reference: row.get(9)?,
        width: row.get(10)?,
        height: row.get(11)?,
        duration_ms: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
        retention_mode: row.get(13)?,
        expires_at_ms: row.get(14)?,
        redaction_state: row.get(16)?,
        redaction_categories: serde_json::from_str(&categories).unwrap_or_default(),
        routing_mode: row.get(18)?,
        provider_ids: serde_json::from_str(&providers).unwrap_or_default(),
        created_at_ms: row.get(19)?,
        latest_transcript: None,
        related_asset_ids: vec![],
    })
}

const SELECT: &str = "SELECT media_asset_id,project_id,task_id,task_run_id,media_kind,mime_type,sha256,byte_length,source_kind,source_reference,width,height,duration_ms,retention_mode,expires_at_ms,redaction_categories_json,redaction_state,provider_ids_json,routing_mode,created_at_ms FROM media_assets";

fn hydrate(
    connection: &rusqlite::Connection,
    mut item: MediaAssetRecord,
) -> Result<MediaAssetRecord, String> {
    item.latest_transcript = latest_transcript(connection, &item.media_asset_id)?;
    let mut statement=connection.prepare("SELECT related_media_asset_id FROM media_asset_relationships WHERE media_asset_id=?1 ORDER BY created_at_ms").map_err(|e|e.to_string())?;
    item.related_asset_ids = statement
        .query_map(params![item.media_asset_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(item)
}

pub fn list(engine: &PersistenceEngine, project_id: &str) -> Result<Vec<MediaAssetRecord>, String> {
    ensure_project(engine, project_id)?;
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement = connection
        .prepare(&format!(
            "{SELECT} WHERE project_id=?1 AND deleted_at_ms IS NULL ORDER BY created_at_ms DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id], from_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|item| hydrate(&connection, item))
        .collect()
}

pub fn get(
    engine: &PersistenceEngine,
    project_id: &str,
    id: &str,
) -> Result<MediaAssetRecord, String> {
    ProjectId::parse(project_id)?;
    crate::p1_contracts::MediaAssetId::parse(id)?;
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let item = connection
        .query_row(
            &format!(
                "{SELECT} WHERE project_id=?1 AND media_asset_id=?2 AND deleted_at_ms IS NULL"
            ),
            params![project_id, id],
            from_row,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Media item was not found.".to_string())?;
    hydrate(&connection, item)
}

pub fn data(
    engine: &PersistenceEngine,
    request: &MediaAssetRequest,
) -> Result<MediaAssetData, String> {
    let record = get(engine, &request.project_id, &request.media_asset_id)?;
    let bytes: Vec<u8> = engine
        .open_connection()
        .map_err(|e| e.to_string())?
        .query_row(
            "SELECT original_blob FROM media_assets WHERE media_asset_id=?1",
            params![request.media_asset_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if crate::foundation::digest::sha256_hex(&bytes) != record.sha256 {
        return Err("Media integrity check failed.".into());
    }
    Ok(MediaAssetData {
        media_asset_id: record.media_asset_id,
        mime_type: record.mime_type,
        data_base64: STANDARD.encode(bytes),
        sha256: record.sha256,
    })
}

pub fn save_transcript(
    engine: &PersistenceEngine,
    request: SaveTranscriptRequest,
) -> Result<TranscriptRecord, String> {
    let asset = get(engine, &request.project_id, &request.media_asset_id)?;
    if asset.media_kind != "audio" {
        return Err("Transcripts can only be attached to audio.".into());
    }
    let text = request.transcript.trim();
    if text.is_empty() || text.chars().count() > 100_000 {
        return Err("The transcript is empty or too long.".into());
    }
    if !matches!(request.route_kind.as_str(), "local" | "provider" | "manual") {
        return Err("The transcript route is invalid.".into());
    }
    if request
        .timestamps
        .iter()
        .any(|v| v.end_ms < v.start_ms || v.text.trim().is_empty())
    {
        return Err("A transcript timestamp is invalid.".into());
    }
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let revision: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(revision),0)+1 FROM media_transcripts WHERE media_asset_id=?1",
            params![request.media_asset_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    connection.execute("INSERT INTO media_transcripts (media_asset_id,revision,transcript,language,confidence,timestamps_json,route_kind,route_label,edited_by_user,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![request.media_asset_id,revision,text,request.language.trim(),request.confidence,serde_json::to_string(&request.timestamps).map_err(|e|e.to_string())?,request.route_kind,request.route_label.trim(),request.edited_by_user,now]).map_err(|e|e.to_string())?;
    latest_transcript(&connection, &asset.media_asset_id)?
        .ok_or_else(|| "Transcript was not saved.".into())
}

pub fn delete(engine: &PersistenceEngine, request: &MediaAssetRequest) -> Result<(), String> {
    get(engine, &request.project_id, &request.media_asset_id)?;
    let changed=engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE media_assets SET deleted_at_ms=?3,original_blob=zeroblob(0) WHERE project_id=?1 AND media_asset_id=?2 AND deleted_at_ms IS NULL",params![request.project_id,request.media_asset_id,crate::foundation::clock::unix_time_ms_i64()]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("Media item was not found.".into());
    }
    Ok(())
}

pub fn sanitize_png(
    engine: &PersistenceEngine,
    request: &MediaAssetRequest,
) -> Result<MediaAssetRecord, String> {
    let source = get(engine, &request.project_id, &request.media_asset_id)?;
    if source.mime_type != "image/png" {
        return Err("Only PNG images can be sanitized in this build.".into());
    }
    let raw = data(engine, request)?;
    let bytes = STANDARD
        .decode(raw.data_base64)
        .map_err(|_| "Media payload is invalid.".to_string())?;
    let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
        .map_err(|_| "Image decode failed.".to_string())?;
    let mut clean = Cursor::new(Vec::new());
    image
        .write_to(&mut clean, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let derivative = ingest(
        engine,
        IngestMediaRequest {
            project_id: source.project_id.clone(),
            task_id: source.task_id.clone(),
            task_run_id: source.task_run_id.clone(),
            source_kind: "generated".into(),
            source_reference: "Metadata removed from source image".into(),
            mime_type: "image/png".into(),
            data_base64: STANDARD.encode(clean.into_inner()),
            retention_mode: source.retention_mode.clone(),
            expires_at_ms: source.expires_at_ms,
            redaction_categories: vec![],
            routing_mode: source.routing_mode.clone(),
            provider_ids: source.provider_ids.clone(),
        },
    )?;
    engine.open_connection().map_err(|e|e.to_string())?.execute("INSERT INTO media_asset_relationships (media_asset_id,related_media_asset_id,relationship,created_at_ms) VALUES (?1,?2,'derivative',?3)",params![source.media_asset_id,derivative.media_asset_id,crate::foundation::clock::unix_time_ms_i64()]).map_err(|e|e.to_string())?;
    Ok(derivative)
}

fn save_interpretation(
    engine: &PersistenceEngine,
    project_id: &str,
    asset_id: &str,
    kind: &str,
    text: &str,
    route: &str,
    edited: bool,
) -> Result<MediaInterpretation, String> {
    let asset = get(engine, project_id, asset_id)?;
    if asset.media_kind != "image" {
        return Err("Image understanding is available only for images.".into());
    }
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 20_000 {
        return Err("The image description is empty or too long.".into());
    }
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let revision: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(revision),0)+1 FROM media_interpretations WHERE media_asset_id=?1",
            params![asset_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    connection.execute("INSERT INTO media_interpretations (media_asset_id,revision,interpretation_kind,text,route_label,edited_by_user,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![asset_id,revision,kind,text,route,edited,now]).map_err(|e|e.to_string())?;
    Ok(MediaInterpretation {
        revision: revision as u64,
        interpretation_kind: kind.into(),
        text: text.into(),
        route_label: route.into(),
        edited_by_user: edited,
        created_at_ms: now,
    })
}

pub fn analyze_image(
    engine: &PersistenceEngine,
    request: &MediaAssetRequest,
) -> Result<MediaInterpretation, String> {
    let asset = get(engine, &request.project_id, &request.media_asset_id)?;
    if asset.mime_type != "image/png" {
        return Err("This image format cannot be understood locally yet.".into());
    }
    let raw = data(engine, request)?;
    let bytes = STANDARD
        .decode(raw.data_base64)
        .map_err(|_| "Media payload is invalid.".to_string())?;
    let path = std::path::PathBuf::from(format!("{}.png", asset.media_asset_id));
    let context = crate::tools::vision::analyze_visual_bytes_for_context(&path, bytes)?;
    save_interpretation(
        engine,
        &request.project_id,
        &request.media_asset_id,
        "local_vision",
        &context.text,
        "On-device vision",
        false,
    )
}

pub fn save_alt_text(
    engine: &PersistenceEngine,
    request: SaveMediaInterpretationRequest,
) -> Result<MediaInterpretation, String> {
    save_interpretation(
        engine,
        &request.project_id,
        &request.media_asset_id,
        "alt_text",
        &request.text,
        "Edited by you",
        true,
    )
}
