use super::*;
use crate::db::PersistenceEngine;

async fn blocking<T: Send + 'static>(
    op: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(op)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn ingest_media_asset(
    request: IngestMediaRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<MediaAssetRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::ingest(&engine, request)).await
}
#[tauri::command]
pub async fn list_media_assets(
    request: MediaProjectRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<MediaAssetRecord>, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::list(&engine, &request.project_id)).await
}
#[tauri::command]
pub async fn get_media_asset_data(
    request: MediaAssetRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<MediaAssetData, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::data(&engine, &request)).await
}
#[tauri::command]
pub async fn save_media_transcript(
    request: SaveTranscriptRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TranscriptRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::save_transcript(&engine, request)).await
}
#[tauri::command]
pub async fn delete_media_asset(
    request: MediaAssetRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::delete(&engine, &request)).await
}
#[tauri::command]
pub async fn sanitize_media_image(
    request: MediaAssetRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<MediaAssetRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::sanitize_png(&engine, &request)).await
}
#[tauri::command]
pub async fn analyze_media_image(
    request: MediaAssetRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<MediaInterpretation, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::analyze_image(&engine, &request)).await
}
#[tauri::command]
pub async fn save_media_alt_text(
    request: SaveMediaInterpretationRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<MediaInterpretation, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::save_alt_text(&engine, request)).await
}
